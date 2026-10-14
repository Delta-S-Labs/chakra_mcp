//! Per-account request-velocity limiting.
//!
//! Fixed 60-second window keyed by `(account, current-minute)`. Redis-backed
//! in production; a `Noop` variant (always-allow) is used when `REDIS_URL` is
//! unset, which keeps local dev and the `sqlx::test` suite free of a Redis
//! dependency. Every Redis error **fails open** (allow + warn): a rate-limiter
//! outage must never take the relay down.

use uuid::Uuid;

/// What the limiter knows after counting a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateOutcome {
    /// Within the limit.
    Allowed(RateInfo),
    /// Over the limit for the rest of this window.
    Limited(RateInfo),
    /// No count: no Redis configured, or a Redis error. Allowed (fail open),
    /// and nothing is reported to the caller.
    Unknown,
}

impl RateOutcome {
    pub fn is_limited(&self) -> bool {
        matches!(self, RateOutcome::Limited(_))
    }

    /// The window's state, when the limiter counted.
    pub fn info(&self) -> Option<RateInfo> {
        match self {
            RateOutcome::Allowed(info) | RateOutcome::Limited(info) => Some(*info),
            RateOutcome::Unknown => None,
        }
    }
}

/// One fixed window's state, for the `RateLimit-*` headers and `Retry-After`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateInfo {
    /// Calls allowed per window.
    pub limit: i32,
    /// Calls left in this window (0 once over).
    pub remaining: i32,
    /// Seconds until this window ends and the count starts again (1..=60).
    pub reset_secs: u32,
}

impl RateInfo {
    fn from_count(count: i64, per_min: i32, unix_secs: i64) -> Self {
        Self {
            limit: per_min,
            remaining: (per_min as i64 - count).clamp(0, per_min.max(0) as i64) as i32,
            reset_secs: window_reset_secs(unix_secs),
        }
    }
}

/// Seconds left in the fixed one-minute window containing `unix_secs`.
fn window_reset_secs(unix_secs: i64) -> u32 {
    (60 - unix_secs.rem_euclid(60)) as u32
}

fn outcome(count: i64, per_min: i32, unix_secs: i64) -> RateOutcome {
    let info = RateInfo::from_count(count, per_min, unix_secs);
    if count > per_min as i64 {
        RateOutcome::Limited(info)
    } else {
        RateOutcome::Allowed(info)
    }
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
    /// `per_min` within the current 60-second window, with the window's
    /// state (the count the window script already returns, so no extra
    /// round trip).
    pub async fn check(&self, account: Uuid, per_min: i32) -> RateOutcome {
        match self {
            RateLimiter::Noop => RateOutcome::Unknown,
            RateLimiter::Redis(pool) => check_redis(pool, account, per_min).await,
            #[cfg(test)]
            RateLimiter::Counting(map) => {
                let now = chrono::Utc::now().timestamp();
                let key = window_key(account, now);
                let mut m = map.lock().unwrap();
                let c = m.entry(key).or_insert(0);
                *c += 1;
                outcome(*c, per_min, now)
            }
        }
    }
}

fn window_key(account: Uuid, unix_secs: i64) -> String {
    let minute = unix_secs.div_euclid(60);
    format!("rl:{account}:{minute}")
}

async fn check_redis(pool: &deadpool_redis::Pool, account: Uuid, per_min: i32) -> RateOutcome {
    // now() is process-clock; regular runtime code (not a workflow script).
    let now = chrono::Utc::now().timestamp();
    let key = window_key(account, now);
    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "redis unavailable for rate check; failing open");
            record_redis_error();
            return RateOutcome::Unknown;
        }
    };
    let script = redis::Script::new(WINDOW_SCRIPT);
    match script.key(&key).invoke_async::<i64>(&mut conn).await {
        Ok(count) => outcome(count, per_min, now),
        Err(e) => {
            tracing::warn!(error = %e, "redis rate check failed; failing open");
            record_redis_error();
            RateOutcome::Unknown
        }
    }
}

/// Count a Redis error that made the limiter fail open.
fn record_redis_error() {
    metrics::counter!(chakramcp_shared::telemetry::names::RATE_LIMITER_ERRORS_TOTAL).increment(1);
}

/// Test-only: a **live** Redis pool for integration tests, or `None` (with a
/// skip note) when no Redis is reachable — so the suite stays green on
/// machines/CI without a Redis service. Point it at a specific instance with
/// `REDIS_TEST_URL`; defaults to `redis://127.0.0.1:6379`. The probe (a PING
/// under a 2s timeout) is what distinguishes "skip, no Redis here" from a real
/// test failure.
#[cfg(test)]
pub(crate) async fn test_redis_pool() -> Option<deadpool_redis::Pool> {
    let url =
        std::env::var("REDIS_TEST_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let pool = match deadpool_redis::Config::from_url(&url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SKIP live-redis test: bad REDIS_TEST_URL {url:?}: {e}");
            return None;
        }
    };
    let probe = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut conn = pool.get().await.map_err(|e| e.to_string())?;
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| e.to_string())
    })
    .await;
    match probe {
        Ok(Ok(_)) => Some(pool),
        other => {
            eprintln!(
                "SKIP live-redis test: no Redis at {url} ({other:?}); \
                 start one with `docker run -d -p 6379:6379 redis:7-alpine` \
                 or set REDIS_TEST_URL"
            );
            None
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

    /// Live Redis: exercises the real `check_redis` path (the Lua
    /// `INCR`+`EXPIRE` window script) that the `Counting` mock stands in for
    /// everywhere else. Skips cleanly when no Redis is reachable.
    #[tokio::test]
    async fn live_redis_fixed_window_limits_and_expires() {
        let Some(pool) = test_redis_pool().await else {
            return;
        };
        let rl = RateLimiter::Redis(pool.clone());
        let acct = Uuid::now_v7();

        // per_min = 3: first three allowed on real Redis, fourth limited.
        assert!(!rl.check(acct, 3).await.is_limited());
        assert!(!rl.check(acct, 3).await.is_limited());
        assert!(!rl.check(acct, 3).await.is_limited());
        assert!(rl.check(acct, 3).await.is_limited());

        // The window key carries a TTL (1..=60s): the fixed window self-expires
        // so minute buckets never leak. This is the EXPIRE branch of
        // WINDOW_SCRIPT — untestable through the in-memory mock.
        let key = window_key(acct, chrono::Utc::now().timestamp());
        let mut conn = pool.get().await.unwrap();
        let ttl: i64 = redis::cmd("TTL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!((1..=60).contains(&ttl), "expected a 1..=60s TTL, got {ttl}");

        // A different account has an independent bucket on the same live Redis.
        let other = Uuid::now_v7();
        assert!(!rl.check(other, 1).await.is_limited());
        assert!(rl.check(other, 1).await.is_limited());
    }

    #[tokio::test]
    async fn noop_always_allows_and_reports_nothing() {
        let rl = RateLimiter::Noop;
        for _ in 0..1000 {
            assert_eq!(rl.check(Uuid::now_v7(), 1).await, RateOutcome::Unknown);
        }
    }

    #[test]
    fn the_window_resets_on_the_minute() {
        assert_eq!(
            window_reset_secs(1_800_000_000),
            60,
            "on the minute: a whole window"
        );
        assert_eq!(window_reset_secs(1_800_000_037), 23);
        assert_eq!(window_reset_secs(1_800_000_059), 1, "never 0");
        assert_eq!(
            window_key(Uuid::nil(), 1_800_000_059),
            window_key(Uuid::nil(), 1_800_000_000)
        );
    }

    #[test]
    fn the_outcome_carries_the_count() {
        let now = 1_800_000_037;
        assert_eq!(
            outcome(1, 3, now),
            RateOutcome::Allowed(RateInfo {
                limit: 3,
                remaining: 2,
                reset_secs: 23
            })
        );
        assert_eq!(
            outcome(3, 3, now),
            RateOutcome::Allowed(RateInfo {
                limit: 3,
                remaining: 0,
                reset_secs: 23
            })
        );
        let over = outcome(9, 3, now);
        assert!(over.is_limited());
        assert_eq!(over.info().unwrap().remaining, 0, "never negative");
    }

    #[tokio::test]
    async fn counting_limits_after_per_min_hits() {
        let rl = counting();
        let acct = Uuid::now_v7();
        // per_min = 3: first three allowed, fourth limited.
        assert!(!rl.check(acct, 3).await.is_limited());
        assert!(!rl.check(acct, 3).await.is_limited());
        assert!(!rl.check(acct, 3).await.is_limited());
        assert!(rl.check(acct, 3).await.is_limited());
    }

    #[tokio::test]
    async fn counting_is_per_account() {
        let rl = counting();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert!(!rl.check(a, 1).await.is_limited());
        assert!(rl.check(a, 1).await.is_limited());
        // b has its own bucket.
        assert!(!rl.check(b, 1).await.is_limited());
    }
}
