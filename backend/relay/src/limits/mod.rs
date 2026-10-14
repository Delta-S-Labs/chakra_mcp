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
pub use rate::{RateInfo, RateLimiter, RateOutcome};

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use chakramcp_shared::error::ApiError;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Why an invocation is refused (or, in shadow mode, would have been), with
/// what the caller needs to know about retrying (credits P6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitOutcome {
    /// Over the per-minute rate limit until the window ends.
    RateLimited { limit: i32, retry_after_secs: u64 },
    /// Out of credits until the next monthly free grant (or a purchase).
    InsufficientCredits {
        retry_after_secs: u64,
        next_grant_at: DateTime<Utc>,
    },
}

impl LimitOutcome {
    fn kind(&self) -> &'static str {
        match self {
            LimitOutcome::RateLimited { .. } => "rate",
            LimitOutcome::InsufficientCredits { .. } => "credits",
        }
    }

    /// The error the REST and MCP surfaces return. `credits_url` is where to
    /// buy credits (chakramcp.com only).
    pub fn into_api_error(self, credits_url: Option<String>) -> ApiError {
        match self {
            LimitOutcome::RateLimited {
                limit,
                retry_after_secs,
            } => ApiError::RateLimited {
                limit,
                retry_after_secs,
            },
            LimitOutcome::InsufficientCredits {
                retry_after_secs,
                next_grant_at,
            } => ApiError::InsufficientCredits {
                retry_after_secs,
                next_grant_at,
                credits_url,
            },
        }
    }
}

/// What an allowed call reports to its caller: the rate window (when the
/// limiter counted) and whether the account is running low on credits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimitSignals {
    pub rate: Option<RateInfo>,
    pub credits_low: bool,
}

pub const RATELIMIT_LIMIT: HeaderName = HeaderName::from_static("ratelimit-limit");
pub const RATELIMIT_REMAINING: HeaderName = HeaderName::from_static("ratelimit-remaining");
pub const RATELIMIT_RESET: HeaderName = HeaderName::from_static("ratelimit-reset");
pub const CHAKRAMCP_CREDITS: HeaderName = HeaderName::from_static("chakramcp-credits");

impl LimitSignals {
    /// Add `RateLimit-Limit/-Remaining/-Reset` and `ChakraMCP-Credits: low`.
    pub fn apply(&self, headers: &mut HeaderMap) {
        if let Some(rate) = self.rate {
            headers.insert(RATELIMIT_LIMIT, HeaderValue::from(rate.limit));
            headers.insert(RATELIMIT_REMAINING, HeaderValue::from(rate.remaining));
            headers.insert(RATELIMIT_RESET, HeaderValue::from(rate.reset_secs));
        }
        if self.credits_low {
            headers.insert(CHAKRAMCP_CREDITS, HeaderValue::from_static("low"));
        }
    }
}

/// The gate's verdict: a refusal, or the signals for an allowed call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitDecision {
    pub refusal: Option<LimitOutcome>,
    pub signals: LimitSignals,
}

/// Where an out-of-credits refusal sends the caller to buy credits: the web
/// app's credits page, on chakramcp.com (managed hosting) only. A self-hosted
/// server may have no web UI at its frontend address, so it sends no link.
pub fn credits_url(managed: bool, frontend_base_url: &str) -> Option<String> {
    managed.then(|| format!("{}/app/credits", frontend_base_url.trim_end_matches('/')))
}

/// 00:00 UTC on the 1st of next month: when the credits worker's monthly
/// grant lands (and the public-invoke quota resets).
pub(crate) fn next_month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::{Datelike, NaiveDate, TimeZone};
    let (y, m) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(y, m, 1)
            .expect("valid date")
            .and_hms_opt(0, 0, 0)
            .expect("valid time"),
    )
}

/// The usage-limit gate. With `enforce` false (shadow mode) an over-limit
/// call is logged as `limit.would_block` and allowed. `surface` labels the
/// logs.
///
/// The credit switch is checked first: it's an in-memory lookup, so an
/// exhausted account is refused without a Redis round trip. Everything
/// here is memory or the one Redis call: no database.
pub async fn enforce(
    limiter: &RateLimiter,
    credits: &CreditCache,
    default_rate_per_min: i32,
    account_id: Uuid,
    enforce: bool,
    surface: &str,
) -> LimitDecision {
    let mut signals = LimitSignals::default();
    if credits.is_blocked(account_id) {
        let now = Utc::now();
        let next_grant_at = next_month_start(now);
        let outcome = LimitOutcome::InsufficientCredits {
            retry_after_secs: (next_grant_at - now).num_seconds().max(1) as u64,
            next_grant_at,
        };
        record_refusal(&outcome, enforce);
        if enforce {
            return LimitDecision {
                refusal: Some(outcome),
                signals,
            };
        }
        would_block(&outcome, account_id, surface);
    } else {
        signals.credits_low = credits.is_low(account_id);
    }
    let per_min = credits.rate_limit_for(account_id, default_rate_per_min);
    let rate = limiter.check(account_id, per_min).await;
    signals.rate = rate.info();
    if let RateOutcome::Limited(info) = rate {
        let outcome = LimitOutcome::RateLimited {
            limit: info.limit,
            retry_after_secs: u64::from(info.reset_secs),
        };
        record_refusal(&outcome, enforce);
        if enforce {
            return LimitDecision {
                refusal: Some(outcome),
                signals,
            };
        }
        would_block(&outcome, account_id, surface);
    }
    LimitDecision {
        refusal: None,
        signals,
    }
}

/// Count a refusal (or, in shadow mode, a would-be refusal).
fn record_refusal(outcome: &LimitOutcome, enforced: bool) {
    let kind = outcome.kind();
    metrics::counter!(
        chakramcp_shared::telemetry::names::LIMIT_REFUSALS_TOTAL,
        "kind" => kind,
        "enforced" => if enforced { "true" } else { "false" }
    )
    .increment(1);
}

fn would_block(outcome: &LimitOutcome, account: Uuid, surface: &str) {
    let kind = outcome.kind();
    tracing::info!(
        event = "limit.would_block",
        kind,
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
    use crate::telemetry::testing::Recorded;

    fn counting_limiter() -> RateLimiter {
        RateLimiter::Counting(Arc::new(Mutex::new(HashMap::new())))
    }

    fn blocking(account: Uuid) -> CreditCache {
        let cache = CreditCache::default();
        cache.replace(HashSet::from([account]), HashMap::new());
        cache
    }

    fn refusals(m: &Recorded, kind: &str, enforced: &str) -> u64 {
        let labels = [("kind", kind), ("enforced", enforced)];
        m.counter(
            chakramcp_shared::telemetry::names::LIMIT_REFUSALS_TOTAL,
            &labels,
        )
    }

    #[tokio::test]
    async fn an_account_with_credits_proceeds() {
        let m = Recorded::start();
        let acct = Uuid::now_v7();
        let decision = enforce(
            &RateLimiter::Noop,
            &CreditCache::default(),
            60,
            acct,
            true,
            "t",
        )
        .await;
        assert_eq!(decision.refusal, None);
        assert_eq!(
            decision.signals,
            LimitSignals::default(),
            "no Redis, not low"
        );
        // The allowed path records nothing.
        for (kind, enforced) in [("credits", "true"), ("rate", "true")] {
            assert_eq!(refusals(&m, kind, enforced), 0);
        }
    }

    #[tokio::test]
    async fn an_exhausted_account_is_refused_when_enforcing() {
        let m = Recorded::start();
        let acct = Uuid::now_v7();
        let decision = enforce(&RateLimiter::Noop, &blocking(acct), 60, acct, true, "t").await;
        let Some(LimitOutcome::InsufficientCredits {
            retry_after_secs,
            next_grant_at,
        }) = decision.refusal
        else {
            panic!("expected a credits refusal, got {decision:?}");
        };
        assert_eq!(next_grant_at, next_month_start(Utc::now()));
        assert!((1..=31 * 86_400).contains(&retry_after_secs));
        assert_eq!(refusals(&m, "credits", "true"), 1);
    }

    #[tokio::test]
    async fn shadow_mode_only_logs() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = blocking(acct);
        // Out of credits and, past 60 calls, over the rate limit too.
        let m = Recorded::start();
        for _ in 0..70 {
            assert_eq!(
                enforce(&limiter, &cache, 60, acct, false, "t")
                    .await
                    .refusal,
                None
            );
        }
        // Counted as would-be refusals, never as enforced ones.
        assert_eq!(refusals(&m, "credits", "false"), 70);
        assert_eq!(refusals(&m, "rate", "false"), 10);
        assert_eq!(refusals(&m, "credits", "true"), 0);
    }

    #[tokio::test]
    async fn the_rate_limit_trips_after_per_min_hits() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        for _ in 0..60 {
            assert_eq!(
                enforce(&limiter, &cache, 60, acct, true, "t").await.refusal,
                None
            );
        }
        assert!(matches!(
            enforce(&limiter, &cache, 60, acct, true, "t").await.refusal,
            Some(LimitOutcome::RateLimited { .. })
        ));
    }

    #[tokio::test]
    async fn a_rate_override_replaces_the_default() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        cache.replace(HashSet::new(), HashMap::from([(acct, 3)]));
        for _ in 0..3 {
            assert_eq!(
                enforce(&limiter, &cache, 60, acct, true, "t").await.refusal,
                None
            );
        }
        assert!(matches!(
            enforce(&limiter, &cache, 60, acct, true, "t").await.refusal,
            Some(LimitOutcome::RateLimited { .. })
        ));
    }

    #[tokio::test]
    async fn an_exhausted_account_never_reaches_the_rate_limiter() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = blocking(acct);
        for _ in 0..100 {
            assert!(matches!(
                enforce(&limiter, &cache, 1, acct, true, "t").await.refusal,
                Some(LimitOutcome::InsufficientCredits { .. })
            ));
        }
        // None of those 100 refusals consumed rate budget.
        let cleared = CreditCache::default();
        assert_eq!(
            enforce(&limiter, &cleared, 1, acct, true, "t")
                .await
                .refusal,
            None
        );
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
                enforce(&limiter, &cache, 60, acct, true, "test-live")
                    .await
                    .refusal,
                None,
                "hit {i} should be under the limit"
            );
        }
        assert!(matches!(
            enforce(&limiter, &cache, 60, acct, true, "test-live")
                .await
                .refusal,
            Some(LimitOutcome::RateLimited { .. })
        ));
    }

    #[tokio::test]
    async fn an_allowed_call_reports_its_window_and_a_low_balance() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        cache.replace_all(
            HashSet::new(),
            HashSet::from([acct]),
            HashMap::from([(acct, 3)]),
        );
        let first = enforce(&limiter, &cache, 60, acct, true, "t").await;
        assert_eq!(first.refusal, None);
        assert!(first.signals.credits_low);
        let rate = first.signals.rate.expect("the limiter counted");
        assert_eq!((rate.limit, rate.remaining), (3, 2));
        assert!((1..=60).contains(&rate.reset_secs));

        let mut headers = HeaderMap::new();
        first.signals.apply(&mut headers);
        assert_eq!(headers[&RATELIMIT_LIMIT], "3");
        assert_eq!(headers[&RATELIMIT_REMAINING], "2");
        assert_eq!(headers[&CHAKRAMCP_CREDITS], "low");
    }

    #[tokio::test]
    async fn a_rate_refusal_says_when_the_window_ends() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        enforce(&limiter, &cache, 1, acct, true, "t").await;
        let decision = enforce(&limiter, &cache, 1, acct, true, "t").await;
        let Some(LimitOutcome::RateLimited {
            limit,
            retry_after_secs,
        }) = decision.refusal
        else {
            panic!("expected a rate refusal, got {decision:?}");
        };
        assert_eq!(limit, 1);
        assert!((1..=60).contains(&retry_after_secs));
        let api = LimitOutcome::RateLimited {
            limit,
            retry_after_secs,
        }
        .into_api_error(None);
        assert!(matches!(api, ApiError::RateLimited { .. }));
    }

    #[tokio::test]
    async fn shadow_mode_reports_an_empty_window() {
        let acct = Uuid::now_v7();
        let limiter = counting_limiter();
        let cache = CreditCache::default();
        enforce(&limiter, &cache, 1, acct, false, "t").await;
        let over = enforce(&limiter, &cache, 1, acct, false, "t").await;
        assert_eq!(over.refusal, None, "shadow mode never refuses");
        assert_eq!(over.signals.rate.unwrap().remaining, 0);
    }

    #[tokio::test]
    async fn a_blocked_account_is_not_also_low() {
        let acct = Uuid::now_v7();
        let cache = CreditCache::default();
        cache.replace_all(HashSet::from([acct]), HashSet::from([acct]), HashMap::new());
        let shadow = enforce(&RateLimiter::Noop, &cache, 60, acct, false, "t").await;
        assert!(!shadow.signals.credits_low);
    }

    #[test]
    fn the_next_grant_is_the_first_of_next_month() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        assert_eq!(
            next_month_start(at("2026-10-15T12:00:00Z")),
            at("2026-11-01T00:00:00Z")
        );
        assert_eq!(
            next_month_start(at("2026-12-31T23:59:59Z")),
            at("2027-01-01T00:00:00Z")
        );
        assert_eq!(
            next_month_start(at("2026-11-01T00:00:00Z")),
            at("2026-12-01T00:00:00Z")
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
