use std::sync::Arc;

use sqlx::PgPool;

use chakramcp_shared::config::SharedConfig;

use crate::limits::RateLimiter;

#[derive(Clone)]
pub struct RelayState {
    pub db: PgPool,
    #[allow(dead_code)]
    pub config: Arc<SharedConfig>,
    /// Per-account request-velocity limiter. Defaults to `Noop` (no Redis);
    /// production injects a Redis-backed one via [`RelayState::with_rate_limiter`].
    /// Read by the invocation surfaces in PR 3.
    #[allow(dead_code)]
    pub rate_limiter: Arc<RateLimiter>,
}

impl RelayState {
    /// Construct with rate limiting disabled (Noop). Keeps the two-arg
    /// signature every existing caller (incl. tests) relies on; production
    /// chains [`with_rate_limiter`](Self::with_rate_limiter).
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
            rate_limiter: Arc::new(RateLimiter::Noop),
        }
    }

    /// Replace the rate limiter (production wiring reads `REDIS_URL`).
    pub fn with_rate_limiter(mut self, limiter: RateLimiter) -> Self {
        self.rate_limiter = Arc::new(limiter);
        self
    }

    pub fn jwt_secret(&self) -> &str {
        &self.config.jwt_secret
    }
}
