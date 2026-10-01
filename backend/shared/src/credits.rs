//! Credit settings and the blocking rule, shared by the relay (which
//! enforces credits) and the app (which shows them to owners and admins).
//! Design: `docs/specs/2026-09-23-credit-ledger-foundation-design.md`.

use std::time::Duration;

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::hosting::env_value;

/// Global credit defaults. Per-account values on `credit_wallets` override
/// the grant and the rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditsConfig {
    /// Whether credits are charged, granted and enforced. The default comes
    /// from the hosting mode (`crate::hosting`): on for chakramcp.com, off
    /// for self-hosted servers. Off, the worker discards queued charges and
    /// nobody is blocked for credits; the rate limit still applies.
    pub enabled: bool,
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
            enabled: true,
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

/// The `server.toml` keys, named after their environment variables.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreditsFile {
    pub credits_default_monthly_free_mc: Option<i64>,
    pub credits_cost_per_invocation_mc: Option<i64>,
    pub limits_default_rate_per_min: Option<i32>,
}

impl CreditsConfig {
    /// From the environment only (the standalone binaries). `enabled` comes
    /// from the hosting settings.
    pub fn from_env(enabled: bool) -> anyhow::Result<Self> {
        Self::from_sources(
            |key| std::env::var(key).ok(),
            &CreditsFile::default(),
            enabled,
        )
    }

    /// Each setting from a non-empty environment value, else the config
    /// file, else the default. An empty value counts as unset (Compose
    /// passes one for every `.env` key left out); a value that is set but
    /// unparseable or not positive, or a tick over five minutes, is an
    /// error. Money settings must never silently fall back.
    pub fn from_sources(
        get: impl Fn(&str) -> Option<String>,
        file: &CreditsFile,
        enabled: bool,
    ) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let sweep_secs = positive(
            &get,
            "CREDITS_SWEEP_INTERVAL_SECS",
            None,
            defaults.sweep_interval.as_secs(),
        )?;
        if sweep_secs > MAX_SWEEP_INTERVAL_SECS {
            bail!(
                "CREDITS_SWEEP_INTERVAL_SECS must be at most {MAX_SWEEP_INTERVAL_SECS} (got {sweep_secs})"
            );
        }
        Ok(Self {
            enabled,
            default_monthly_free_mc: positive(
                &get,
                "CREDITS_DEFAULT_MONTHLY_FREE_MC",
                file.credits_default_monthly_free_mc,
                defaults.default_monthly_free_mc,
            )?,
            cost_per_invocation_mc: positive(
                &get,
                "CREDITS_COST_PER_INVOCATION_MC",
                file.credits_cost_per_invocation_mc,
                defaults.cost_per_invocation_mc,
            )?,
            sweep_interval: Duration::from_secs(sweep_secs),
            default_rate_per_min: positive(
                &get,
                "LIMITS_DEFAULT_RATE_PER_MIN",
                file.limits_default_rate_per_min,
                defaults.default_rate_per_min,
            )?,
        })
    }

    /// The balance below which an account is blocked: one invocation's
    /// cost, or, with credits off, a balance no wallet can have, so the
    /// switch refresh blocks nobody.
    pub fn block_below_mc(&self) -> i64 {
        if self.enabled {
            self.cost_per_invocation_mc
        } else {
            i64::MIN
        }
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

fn positive<T>(
    get: &impl Fn(&str) -> Option<String>,
    key: &str,
    from_file: Option<T>,
    default: T,
) -> anyhow::Result<T>
where
    T: std::str::FromStr + PartialOrd + Default + std::fmt::Display,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let value: T = match env_value(get, key) {
        Some(raw) => raw
            .parse()
            .with_context(|| format!("{key}={raw:?} is not a valid number"))?,
        None => match from_file {
            Some(value) => value,
            None => return Ok(default),
        },
    };
    if value <= T::default() {
        bail!("{key} must be greater than 0 (got {value})");
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
        CreditsConfig::from_sources(|key| env.get(key).cloned(), &CreditsFile::default(), true)
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
    fn empty_values_count_as_unset_and_the_file_comes_second() {
        let cfg = from(&[
            ("CREDITS_DEFAULT_MONTHLY_FREE_MC", ""),
            ("CREDITS_COST_PER_INVOCATION_MC", "  "),
        ])
        .unwrap();
        assert_eq!(cfg, CreditsConfig::default());

        let file = CreditsFile {
            credits_default_monthly_free_mc: Some(5_000),
            credits_cost_per_invocation_mc: Some(10),
            limits_default_rate_per_min: Some(30),
        };
        let lookup = |key: &str| (key == "LIMITS_DEFAULT_RATE_PER_MIN").then(|| "90".to_owned());
        let cfg = CreditsConfig::from_sources(lookup, &file, false).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.default_monthly_free_mc, 5_000, "from the file");
        assert_eq!(cfg.cost_per_invocation_mc, 10, "from the file");
        assert_eq!(cfg.default_rate_per_min, 90, "the environment wins");

        let bad = CreditsFile {
            credits_cost_per_invocation_mc: Some(0),
            ..Default::default()
        };
        assert!(CreditsConfig::from_sources(|_| None, &bad, true).is_err());
    }

    #[test]
    fn with_credits_off_nobody_is_blocked() {
        let on = CreditsConfig::default();
        assert_eq!(on.block_below_mc(), on.cost_per_invocation_mc);
        let off = CreditsConfig {
            enabled: false,
            ..on
        };
        assert_eq!(off.block_below_mc(), i64::MIN);
    }

    #[test]
    fn the_tick_is_capped() {
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", "300")]).is_ok());
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", "301")]).is_err());
        // Would overflow `Duration * 3` if it got through.
        assert!(from(&[("CREDITS_SWEEP_INTERVAL_SECS", &u64::MAX.to_string())]).is_err());
    }
}
