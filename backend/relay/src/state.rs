use std::sync::Arc;

use sqlx::PgPool;

use chakramcp_shared::config::SharedConfig;

use crate::compliance::ComplianceChecker;
use crate::limits::RateLimiter;

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
}

impl RelayState {
    /// Construct with rate limiting disabled (Noop) and enforcement off
    /// (shadow). Keeps the two-arg signature every existing caller (incl.
    /// tests) relies on; production chains the builders below.
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
            rate_limiter: Arc::new(RateLimiter::Noop),
            limits_enforce: false,
            compliance: None,
        }
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

    /// Install (or clear) the System One compliance checker.
    pub fn with_compliance(mut self, checker: Option<ComplianceChecker>) -> Self {
        self.compliance = checker.map(Arc::new);
        self
    }

    pub fn jwt_secret(&self) -> &str {
        &self.config.jwt_secret
    }
}
