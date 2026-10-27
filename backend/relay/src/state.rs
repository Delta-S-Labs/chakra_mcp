use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::config::SharedConfig;

use crate::compliance::ComplianceChecker;
use crate::events::UsageRecorder;
use chakramcp_shared::error::ApiError;

use crate::limits::{CreditCache, CreditsConfig, LimitDecision, LimitOutcome, RateLimiter};

#[derive(Clone)]
pub struct RelayState {
    pub db: PgPool,
    #[allow(dead_code)]
    pub config: Arc<SharedConfig>,
    /// Per-account request-velocity limiter. Defaults to `Noop` (no Redis);
    /// production injects a Redis-backed one via [`RelayState::with_rate_limiter`].
    pub rate_limiter: Arc<RateLimiter>,
    /// When false (the default), usage-limit checks run in **shadow mode**:
    /// an over-limit call is logged as a would-block but still allowed. When
    /// true, over-limit calls are actually denied. Set from `LIMITS_ENFORCE`
    /// at startup; tests default to shadow.
    pub limits_enforce: bool,
    /// System One compliance checker (TypeSafe). `None` = checks off, the
    /// default; production wiring reads `SYSTEM_ONE_CHECKS` +
    /// `TYPESAFE_AI_*` via [`ComplianceChecker::from_env`].
    pub compliance: Option<Arc<ComplianceChecker>>,
    /// Usage-event recorder. Records nothing by default; production attaches
    /// one via [`RelayState::with_usage_recorder`] whose background writer
    /// does the DB work, so no request waits on usage metering.
    pub usage: UsageRecorder,
    /// Credit switches (who's out of credits, per-account rate overrides),
    /// kept current by the credits worker. The default was never refreshed,
    /// so nobody is blocked; production shares one with the worker via
    /// [`RelayState::with_credit_cache`].
    pub credit_cache: Arc<CreditCache>,
    /// Credit defaults (grant, cost, tick, default rate limit).
    pub credits: CreditsConfig,
    /// Where an out-of-credits refusal points the caller to buy credits:
    /// `{frontend}/app/credits` on chakramcp.com, `None` on a self-hosted
    /// server (which may have no web UI there). Built once at startup.
    pub credits_url: Option<String>,
    /// Inbox pulls recorded in memory and flushed to `agent_heartbeats`, for
    /// the directory's "online" dot (house agents spec §3). Production
    /// spawns the flusher; tests flush by hand.
    pub heartbeats: crate::heartbeats::Heartbeats,
    /// The account whose agents the directory features first
    /// (`DISCOVERY_FEATURED_ACCOUNT`, chakramcp.com's house agents).
    pub featured_account: Option<String>,
    /// How many days of usage events are kept (`USAGE_RETENTION_DAYS`);
    /// `None` keeps them forever. Production also spawns the pruner.
    pub usage_retention_days: Option<u32>,
    /// The signing key for consent tokens, cached (owner consent §6).
    pub consent_signer: Arc<crate::consent::signer::ConsentSigner>,
    /// Which agents each credential may act as, for 30 s (MCP and discovery
    /// spec §4.2): scope checks on the invoke path read only memory.
    pub scope_cache: Arc<crate::scope_cache::ScopeCache>,
    /// Calls settings (`CALLS_*`); off by default.
    pub calls: crate::calls::CallsConfig,
    /// The TypeSafe client for call memory (calls spec §8): set when
    /// `CALLS_MEMORY_ENABLED` is on and a key is configured.
    pub memory: Option<Arc<ComplianceChecker>>,
    /// MCP settings (`MCP_*`, `TRUST_FORWARDED_FOR`); defaults until the
    /// binary reads them (`with_mcp`).
    pub mcp: crate::mcp::McpConfig,
    /// Slots for MCP tool calls that wait for a result (`MCP_MAX_WAITERS`).
    pub waiters: Arc<tokio::sync::Semaphore>,
}

impl RelayState {
    /// Construct with rate limiting disabled (Noop), enforcement off
    /// (shadow), and credit switches that block nobody. Keeps the two-arg
    /// signature every existing caller (incl. tests) relies on; production
    /// chains the builders below.
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        let consent_signer = Arc::new(crate::consent::signer::ConsentSigner::new(db.clone()));
        Self {
            db,
            config: Arc::new(config),
            rate_limiter: Arc::new(RateLimiter::Noop),
            limits_enforce: false,
            compliance: None,
            usage: UsageRecorder::noop(),
            credit_cache: Arc::new(CreditCache::default()),
            credits: CreditsConfig::default(),
            credits_url: None,
            heartbeats: crate::heartbeats::Heartbeats::default(),
            featured_account: None,
            usage_retention_days: Some(crate::retention::DEFAULT_USAGE_RETENTION_DAYS),
            consent_signer,
            scope_cache: Arc::default(),
            // Off until the binary reads the settings (`with_calls`).
            calls: crate::calls::CallsConfig {
                enabled: false,
                ..Default::default()
            },
            memory: None,
            mcp: crate::mcp::McpConfig::default(),
            waiters: Arc::new(tokio::sync::Semaphore::new(
                crate::mcp::McpConfig::default().max_waiters,
            )),
        }
    }

    /// The usage-limit gate for `account_id` on `surface`: a refusal, or the
    /// signals an allowed call reports. In-memory only (plus the Redis rate
    /// check) — see [`crate::limits::enforce`].
    pub async fn enforce_limits(&self, account_id: Uuid, surface: &str) -> LimitDecision {
        crate::limits::enforce(
            &self.rate_limiter,
            &self.credit_cache,
            self.credits.default_rate_per_min,
            account_id,
            self.limits_enforce,
            surface,
        )
        .await
    }

    /// The REST/MCP error for a usage-limit refusal.
    pub fn refusal_error(&self, outcome: LimitOutcome) -> ApiError {
        outcome.into_api_error(self.credits_url.clone())
    }

    /// The calls settings (spec §14), read from the environment.
    /// MCP settings, and as many waiting slots as they allow.
    pub fn with_mcp(mut self, mcp: crate::mcp::McpConfig) -> Self {
        self.waiters = Arc::new(tokio::sync::Semaphore::new(mcp.max_waiters));
        self.mcp = mcp;
        self
    }

    pub fn with_calls(mut self, calls: crate::calls::CallsConfig) -> Self {
        self.calls = calls;
        self
    }

    /// Call memory's client: from the environment when memory is on.
    pub fn with_memory(mut self, memory: Option<ComplianceChecker>) -> Self {
        self.memory = memory.map(Arc::new);
        self
    }

    /// Point out-of-credits refusals at the web app's credits page (managed
    /// hosting only).
    pub fn with_credits_url(mut self, credits_url: Option<String>) -> Self {
        self.credits_url = credits_url;
        self
    }

    /// Feature this account's agents first in the directory. Empty or
    /// unset features nobody.
    pub fn with_featured_account(mut self, slug: Option<String>) -> Self {
        self.featured_account = slug.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        self
    }

    /// Keep usage events for this many days (`None`: forever). Reported by
    /// the usage endpoint; the pruner is spawned separately.
    pub fn with_usage_retention_days(mut self, days: Option<u32>) -> Self {
        self.usage_retention_days = days;
        self
    }

    /// Share the credit switches the worker keeps current.
    pub fn with_credit_cache(mut self, cache: Arc<CreditCache>) -> Self {
        self.credit_cache = cache;
        self
    }

    /// Set the credit defaults (production reads them from the environment).
    pub fn with_credits_config(mut self, credits: CreditsConfig) -> Self {
        self.credits = credits;
        self
    }

    /// Replace the rate limiter (production wiring reads `REDIS_URL`).
    pub fn with_rate_limiter(mut self, limiter: RateLimiter) -> Self {
        self.rate_limiter = Arc::new(limiter);
        self
    }

    /// Turn usage-limit enforcement on (off = shadow mode). Production
    /// wiring reads `LIMITS_ENFORCE`.
    pub fn with_limits_enforce(mut self, on: bool) -> Self {
        self.limits_enforce = on;
        self
    }

    /// Attach the usage recorder (production: [`UsageRecorder::spawn`]).
    pub fn with_usage_recorder(mut self, recorder: UsageRecorder) -> Self {
        self.usage = recorder;
        self
    }

    /// Install (or clear) the System One compliance checker.
    pub fn with_compliance(mut self, checker: Option<ComplianceChecker>) -> Self {
        self.compliance = checker.map(Arc::new);
        self
    }

    /// [`Self::with_compliance`] with a client shared with the app (the
    /// combined server builds it once).
    pub fn with_compliance_shared(mut self, checker: Option<Arc<ComplianceChecker>>) -> Self {
        self.compliance = checker;
        self
    }

    pub fn jwt_secret(&self) -> &str {
        &self.config.jwt_secret
    }
}
