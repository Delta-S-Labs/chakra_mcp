//! Per-account request-velocity limiting.
//!
//! Fixed 60-second window keyed by `(account, current-minute)`. Redis-backed
//! in production; a `Noop` variant (always-allow) is used when `REDIS_URL` is
//! unset, which keeps local dev and the `sqlx::test` suite free of a Redis
//! dependency. Every Redis error **fails open** (allow + warn): a rate-limiter
//! outage must never take the relay down.

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateOutcome {
    Allowed,
    Limited,
}

/// Atomic `INCR` + first-hit `EXPIRE`, so a minute bucket always carries a TTL
/// (no leaked keys) even under concurrency.
const WINDOW_SCRIPT: &str =
    "local c = redis.call('INCR', KEYS[1]); if c == 1 then redis.call('EXPIRE', KEYS[1], 60) end; return c";

/// A rate limiter. Concrete enum rather than a `dyn` trait so inherent
/// `async fn` works without `async-trait`, while staying fully mockable.
#[derive(Clone)]
pub enum RateLimiter {
    /// Production: a pooled Redis connection.
    Redis(deadpool_redis::Pool),
    /// No Redis configured — always allow (fail-open). Local dev + tests.
    Noop,
    /// Deterministic in-memory counter for tests (same fixed-window logic).
    #[cfg(test)]
    Counting(std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, i64>>>),
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateLimiter::Redis(_) => f.write_str("RateLimiter::Redis"),
            RateLimiter::Noop => f.write_str("RateLimiter::Noop"),
            #[cfg(test)]
            RateLimiter::Counting(_) => f.write_str("RateLimiter::Counting"),
        }
    }
}

impl RateLimiter {
    /// Build from an optional `REDIS_URL`. Unset/empty → `Noop` (fail-open);
    /// a pool-construction error also degrades to `Noop` with an error log,
    /// so a misconfigured Redis can't stop the relay from starting.
    pub fn from_redis_url(url: Option<&str>) -> Self {
        match url.map(str::trim).filter(|u| !u.is_empty()) {
            None => RateLimiter::Noop,
            Some(u) => match deadpool_redis::Config::from_url(u)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            {
                Ok(pool) => RateLimiter::Redis(pool),
                Err(e) => {
                    tracing::error!(error = %e, "redis pool init failed; rate limiting disabled (fail-open)");
                    RateLimiter::Noop
                }
            },
        }
    }

    /// Record one hit for `account` and report whether it is now over
    /// `per_min` within the current 60-second window.
    pub async fn check(&self, account: Uuid, per_min: i32) -> RateOutcome {
        match self {
            RateLimiter::Noop => RateOutcome::Allowed,
            RateLimiter::Redis(pool) => check_redis(pool, account, per_min).await,
            #[cfg(test)]
            RateLimiter::Counting(map) => {
                let key = window_key(account);
                let mut m = map.lock().unwrap();
                let c = m.entry(key).or_insert(0);
                *c += 1;
                if *c > per_min as i64 {
                    RateOutcome::Limited
                } else {
                    RateOutcome::Allowed
                }
            }
        }
    }
}

fn window_key(account: Uuid) -> String {
    // now() is process-clock; regular runtime code (not a workflow script).
    let minute = chrono::Utc::now().timestamp() / 60;
    format!("rl:{account}:{minute}")
}

async fn check_redis(pool: &deadpool_redis::Pool, account: Uuid, per_min: i32) -> RateOutcome {
    let key = window_key(account);
    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "redis unavailable for rate check; failing open");
            return RateOutcome::Allowed;
        }
    };
    let script = redis::Script::new(WINDOW_SCRIPT);
    match script.key(&key).invoke_async::<i64>(&mut conn).await {
        Ok(count) => {
            if count > per_min as i64 {
                RateOutcome::Limited
            } else {
                RateOutcome::Allowed
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "redis rate check failed; failing open");
            RateOutcome::Allowed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counting() -> RateLimiter {
        RateLimiter::Counting(std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::new(),
        )))
    }

    #[tokio::test]
    async fn noop_always_allows() {
        let rl = RateLimiter::Noop;
        for _ in 0..1000 {
            assert_eq!(rl.check(Uuid::now_v7(), 1).await, RateOutcome::Allowed);
        }
    }

    #[tokio::test]
    async fn counting_limits_after_per_min_hits() {
        let rl = counting();
        let acct = Uuid::now_v7();
        // per_min = 3: first three allowed, fourth limited.
        assert_eq!(rl.check(acct, 3).await, RateOutcome::Allowed);
        assert_eq!(rl.check(acct, 3).await, RateOutcome::Allowed);
        assert_eq!(rl.check(acct, 3).await, RateOutcome::Allowed);
        assert_eq!(rl.check(acct, 3).await, RateOutcome::Limited);
    }

    #[tokio::test]
    async fn counting_is_per_account() {
        let rl = counting();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert_eq!(rl.check(a, 1).await, RateOutcome::Allowed);
        assert_eq!(rl.check(a, 1).await, RateOutcome::Limited);
        // b has its own bucket.
        assert_eq!(rl.check(b, 1).await, RateOutcome::Allowed);
    }
}
