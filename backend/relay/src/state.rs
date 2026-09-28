use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::config::SharedConfig;

use crate::compliance::ComplianceChecker;
use crate::events::UsageRecorder;
use crate::limits::{CreditCache, CreditsConfig, LimitOutcome, RateLimiter};

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
}

impl RelayState {
    /// Construct with rate limiting disabled (Noop), enforcement off
    /// (shadow), and credit switches that block nobody. Keeps the two-arg
    /// signature every existing caller (incl. tests) relies on; production
    /// chains the builders below.
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
            rate_limiter: Arc::new(RateLimiter::Noop),
            limits_enforce: false,
            compliance: None,
            usage: UsageRecorder::noop(),
            credit_cache: Arc::new(CreditCache::default()),
            credits: CreditsConfig::default(),
        }
    }

    /// The usage-limit gate for `account_id` on `surface`: the outcome to
    /// refuse with, or `None` to proceed. In-memory only (plus the Redis
    /// rate check) — see [`crate::limits::enforce`].
    pub async fn enforce_limits(&self, account_id: Uuid, surface: &str) -> Option<LimitOutcome> {
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

    pub fn jwt_secret(&self) -> &str {
        &self.config.jwt_secret
    }
}
