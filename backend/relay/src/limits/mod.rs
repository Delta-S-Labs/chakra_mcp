//! Usage limits: a per-account rate limit (Redis) and credits.
//!
//! Design: `docs/specs/2026-09-23-credit-ledger-foundation-design.md`.
//! Credits replaced the per-plan monthly quota of
//! `docs/specs/2026-07-23-usage-quotas-rate-limiting-design.md`.
//!
//! [`enforce`] is the one gate every invocation surface calls. It never
//! touches the database: the credit switch is an in-memory lookup kept
//! current by [`credits::worker`], and the rate limit is one Redis call
//! (fail-open).

pub mod credits;
pub mod rate;

#[cfg(test)]
mod credits_schema_tests;

pub use credits::{CreditCache, CreditsConfig};
pub use rate::{RateLimiter, RateOutcome};

use uuid::Uuid;

/// Why an invocation is refused (or, in shadow mode, would have been).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitOutcome {
    /// Over the per-minute rate limit.
    RateLimited,
    /// Out of credits.
    InsufficientCredits,
}

/// Map a refusal to the HTTP error the REST and MCP surfaces return.
impl From<LimitOutcome> for chakramcp_shared::error::ApiError {
    fn from(outcome: LimitOutcome) -> Self {
        use chakramcp_shared::error::ApiError;
        match outcome {
            LimitOutcome::RateLimited => ApiError::RateLimited,
            LimitOutcome::InsufficientCredits => ApiError::InsufficientCredits,
        }
    }
}

/// The usage-limit gate: the outcome to refuse with, or `None` to proceed.
/// With `enforce` false (shadow mode) an over-limit call is logged as
/// `limit.would_block` and allowed. `surface` labels the logs.
///
/// The credit switch is checked first: it's an in-memory lookup, so an
/// exhausted account is refused without a Redis round trip.
pub async fn enforce(
    limiter: &RateLimiter,
    credits: &CreditCache,
    default_rate_per_min: i32,
    account_id: Uuid,
    enforce: bool,
    surface: &str,
) -> Option<LimitOutcome> {
    if credits.is_blocked(account_id) {
        if enforce {
            return Some(LimitOutcome::InsufficientCredits);
        }
        would_block(LimitOutcome::InsufficientCredits, account_id, surface);
    }
    let per_min = credits.rate_limit_for(account_id, default_rate_per_min);
    if limiter.check(account_id, per_min).await == RateOutcome::Limited {
        if enforce {
            return Some(LimitOutcome::RateLimited);
        }
        would_block(LimitOutcome::RateLimited, account_id, surface);
    }
    None
}

fn would_block(kind: LimitOutcome, account: Uuid, surface: &str) {
    tracing::info!(
        event = "limit.would_block",
        ?kind,
        %account,
        surface,
        "usage limit would block (shadow mode)"
    );
}

/// Parse the `LIMITS_ENFORCE` env value: truthy → enforce, anything else
/// (incl. unset) → shadow mode.
pub fn enforce_flag(val: Option<&str>) -> bool {
    matches!(
        val.map(|s| s.trim().to_lowercase()).as_deref(),
        Some("true" | "1" | "yes" | "on")
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    use super::*;

    fn counting_limiter() -> RateLimiter {
        RateLimiter::Counting(Arc::new(Mutex::new(HashMap::new())))
    }

    fn blocking(account: Uuid) -> CreditCache {
        let cache = CreditCache::default();
        cache.replace(HashSet::from([account]), HashMap::new());
        cache
    }

    #[tokio::test]
    async fn an_account_with_credits_proceeds() {
        let acct = Uuid::now_v7();
        let outcome = enforce(
            &RateLimiter::Noop,
            &CreditCache::default(),
            60,
            acct,
            true,
            "t",
        )
        .await;
        assert_eq!(outcome, None);
    }

    #[tokio::test]
    async fn an_exhausted_account_is_refused_when_enforcing() {
        let acct = Uuid::now_v7();
        let outcome = enforce(&RateLimiter::Noop, &blocking(acct), 60, acct, true, "t").await;
        assert_eq!(outcome, Some(LimitOutcome::InsufficientCredits));
    }

    #[tokio::test]
    async fn shadow_mode_only_logs() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = blocking(acct);
        // Out of credits and, past 60 calls, over the rate limit too.
        for _ in 0..70 {
            assert_eq!(enforce(&limiter, &cache, 60, acct, false, "t").await, None);
        }
    }

    #[tokio::test]
    async fn the_rate_limit_trips_after_per_min_hits() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        for _ in 0..60 {
            assert_eq!(enforce(&limiter, &cache, 60, acct, true, "t").await, None);
        }
        assert_eq!(
            enforce(&limiter, &cache, 60, acct, true, "t").await,
            Some(LimitOutcome::RateLimited)
        );
    }

    #[tokio::test]
    async fn a_rate_override_replaces_the_default() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        cache.replace(HashSet::new(), HashMap::from([(acct, 3)]));
        for _ in 0..3 {
            assert_eq!(enforce(&limiter, &cache, 60, acct, true, "t").await, None);
        }
        assert_eq!(
            enforce(&limiter, &cache, 60, acct, true, "t").await,
            Some(LimitOutcome::RateLimited)
        );
    }

    #[tokio::test]
    async fn an_exhausted_account_never_reaches_the_rate_limiter() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = blocking(acct);
        for _ in 0..100 {
            assert_eq!(
                enforce(&limiter, &cache, 1, acct, true, "t").await,
                Some(LimitOutcome::InsufficientCredits)
            );
        }
        // None of those 100 refusals consumed rate budget.
        let cleared = CreditCache::default();
        assert_eq!(enforce(&limiter, &cleared, 1, acct, true, "t").await, None);
    }

    /// The rate limit over a real Redis. Skips cleanly without one.
    #[tokio::test]
    async fn enforce_denies_rate_live_redis() {
        let Some(redis_pool) = rate::test_redis_pool().await else {
            return;
        };
        let limiter = RateLimiter::Redis(redis_pool);
        let cache = CreditCache::default();
        let acct = Uuid::now_v7();
        for i in 0..60 {
            assert_eq!(
                enforce(&limiter, &cache, 60, acct, true, "test-live").await,
                None,
                "hit {i} should be under the limit"
            );
        }
        assert_eq!(
            enforce(&limiter, &cache, 60, acct, true, "test-live").await,
            Some(LimitOutcome::RateLimited)
        );
    }

    #[test]
    fn enforce_flag_accepts_only_truthy_values() {
        for on in ["true", "1", "yes", "on", " TRUE "] {
            assert!(enforce_flag(Some(on)), "{on:?}");
        }
        for off in ["false", "0", "", "nope"] {
            assert!(!enforce_flag(Some(off)), "{off:?}");
        }
        assert!(!enforce_flag(None));
    }
}
