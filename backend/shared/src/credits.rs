//! Credit settings and the blocking rule, shared by the relay (which
//! enforces credits) and the app (which shows them to owners and admins).
//! Design: `docs/specs/2026-09-23-credit-ledger-foundation-design.md`.

use std::time::Duration;

use anyhow::{bail, Context};

/// Global credit defaults. Per-account values on `credit_wallets` override
/// the grant and the rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditsConfig {
    /// Monthly free grant for wallets with no override (milli-credits).
    pub default_monthly_free_mc: i64,
    /// Flat cost of one accepted invocation (milli-credits).
    pub cost_per_invocation_mc: i64,
    /// How often the worker charges and grants, and (separately) refreshes
    /// the switches.
    pub sweep_interval: Duration,
    /// Rate limit for wallets with no override (invocations per minute).
    pub default_rate_per_min: i32,
}

impl Default for CreditsConfig {
    fn default() -> Self {
        Self {
            // 100 credits = 1,000 invocations at 0.1 credit each: today's free tier.
            default_monthly_free_mc: 100_000,
            cost_per_invocation_mc: 100,
            sweep_interval: Duration::from_secs(5),
            default_rate_per_min: 60,
        }
    }
}

/// Longest allowed worker tick. Anything slower leaves the switches minutes
/// behind, and the fail-open window is a multiple of it.
const MAX_SWEEP_INTERVAL_SECS: u64 = 300;

impl CreditsConfig {
    /// Read from the environment. Unset → default; set but unparseable or
    /// not positive (or a tick over five minutes) → error. Money settings
    /// must never silently fall back.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let sweep_secs = positive(
            &get,
            "CREDITS_SWEEP_INTERVAL_SECS",
            defaults.sweep_interval.as_secs(),
        )?;
        if sweep_secs > MAX_SWEEP_INTERVAL_SECS {
            bail!(
                "CREDITS_SWEEP_INTERVAL_SECS must be at most {MAX_SWEEP_INTERVAL_SECS} (got {sweep_secs})"
            );
        }
        Ok(Self {
            default_monthly_free_mc: positive(
                &get,
                "CREDITS_DEFAULT_MONTHLY_FREE_MC",
                defaults.default_monthly_free_mc,
            )?,
            cost_per_invocation_mc: positive(
                &get,
                "CREDITS_COST_PER_INVOCATION_MC",
                defaults.cost_per_invocation_mc,
            )?,
            sweep_interval: Duration::from_secs(sweep_secs),
            default_rate_per_min: positive(
                &get,
                "LIMITS_DEFAULT_RATE_PER_MIN",
                defaults.default_rate_per_min,
            )?,
        })
    }

    /// After this long without a refresh the switches are treated as
    /// unknown and nobody is blocked (fail open), rather than freezing
    /// whoever was blocked when the worker stopped.
    pub fn stale_after(&self) -> Duration {
        self.sweep_interval.saturating_mul(3)
    }
}

/// Whether an account in this state is refused: it can't afford one more
/// invocation, isn't unlimited, and has had its first monthly grant (a
/// wallet created moments ago mustn't be blocked before that lands). The
/// relay's switch refresh (`relay::limits::credits::cache`) applies the
/// same rule in SQL.
pub fn is_blocked(
    balance_mc: i64,
    granted: bool,
    unlimited: bool,
    cost_per_invocation_mc: i64,
) -> bool {
    !unlimited && granted && balance_mc < cost_per_invocation_mc
}

fn positive<T>(get: &impl Fn(&str) -> Option<String>, key: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr + PartialOrd + Default,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let Some(raw) = get(key) else {
        return Ok(default);
    };
    let value: T = raw
        .trim()
        .parse()
        .with_context(|| format!("{key}={raw:?} is not a valid number"))?;
    if value <= T::default() {
        bail!("{key} must be greater than 0 (got {raw:?})");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn from(pairs: &[(&str, &str)]) -> anyhow::Result<CreditsConfig> {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        CreditsConfig::from_lookup(|key| env.get(key).cloned())
    }

    #[test]
    fn unset_values_use_the_defaults() {
        let cfg = from(&[]).unwrap();
        assert_eq!(cfg, CreditsConfig::default());
        assert_eq!(cfg.default_monthly_free_mc, 100_000);
        assert_eq!(cfg.cost_per_invocation_mc, 100);
        assert_eq!(cfg.stale_after(), Duration::from_secs(15));
    }

    #[test]
    fn explicit_values_are_used() {
        let cfg = from(&[
            ("CREDITS_DEFAULT_MONTHLY_FREE_MC", "250000"),
            ("CREDITS_COST_PER_INVOCATION_MC", " 50 "),
            ("CREDITS_SWEEP_INTERVAL_SECS", "2"),
            ("LIMITS_DEFAULT_RATE_PER_MIN", "600"),
        ])
        .unwrap();
        assert_eq!(cfg.default_monthly_free_mc, 250_000);
        assert_eq!(cfg.cost_per_invocation_mc, 50);
        assert_eq!(cfg.sweep_interval, Duration::from_secs(2));
        assert_eq!(cfg.default_rate_per_min, 600);
    }

    #[test]
    fn bad_values_fail_instead_of_falling_back() {
        assert!(from(&[("CREDITS_COST_PER_INVOCATION_MC", "abc")]).is_err());
        assert!(from(&[("CREDITS_COST_PER_INVOCATION_MC", "-5")]).is_err());
        // A zero interval would panic tokio's interval timer.
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", "0")]).is_err());
        assert!(from(&[("LIMITS_DEFAULT_RATE_PER_MIN", "0")]).is_err());
    }

    #[test]
    fn blocked_only_when_granted_limited_and_short() {
        let cost = 100;
        assert!(is_blocked(99, true, false, cost));
        assert!(is_blocked(-500, true, false, cost));
        assert!(!is_blocked(100, true, false, cost), "can pay for one more");
        assert!(!is_blocked(-500, true, true, cost), "unlimited");
        assert!(!is_blocked(-100, false, false, cost), "never granted");
    }

    #[test]
    fn the_tick_is_capped() {
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", "300")]).is_ok());
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", "301")]).is_err());
        // Would overflow `Duration * 3` if it got through.
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", &u64::MAX.to_string())]).is_err());
    }
}
