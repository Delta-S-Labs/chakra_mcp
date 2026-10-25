//! Calls settings (spec §14), from the environment.

use anyhow::{bail, Context};
use chakramcp_shared::hosting::{env_value, parse_bool};

/// How long a call's content may be kept, at most, unless the operator
/// lowers it.
const DEFAULT_MAX_TTL_DAYS: u32 = 365;
const DEFAULT_TTL_DAYS: u32 = 30;
const DEFAULT_MAX_ACTIVE_PER_AGENT: i64 = 20;
const DEFAULT_MAX_MESSAGES: i32 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallsConfig {
    /// The whole feature. Off: no calls routes (404) and no workers.
    pub enabled: bool,
    /// The longest content TTL a caller may ask for.
    pub max_ttl_secs: i64,
    /// The TTL when the invite doesn't set one.
    pub default_ttl_secs: i64,
    /// Calls an agent may hold at once (spec §3.2).
    pub max_active_per_agent: i64,
    /// Messages one call may hold (spec §3.4).
    pub max_messages: i32,
}

impl Default for CallsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_ttl_secs: days(DEFAULT_MAX_TTL_DAYS),
            default_ttl_secs: days(DEFAULT_TTL_DAYS),
            max_active_per_agent: DEFAULT_MAX_ACTIVE_PER_AGENT,
            max_messages: DEFAULT_MAX_MESSAGES,
        }
    }
}

impl CallsConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok())
    }

    /// Each setting from a non-empty value, else its default. A value that
    /// is set but unparseable or out of range is an error.
    pub fn from_sources(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let enabled = match env_value(&get, "CALLS_ENABLED") {
            Some(raw) => parse_bool("CALLS_ENABLED", &raw)?,
            None => false,
        };
        let max_ttl_days = number(&get, "CALLS_MAX_TTL_DAYS", DEFAULT_MAX_TTL_DAYS, 1, 3650)?;
        let default_ttl_days = number(&get, "CALLS_DEFAULT_TTL_DAYS", DEFAULT_TTL_DAYS, 0, 3650)?;
        if default_ttl_days > max_ttl_days {
            bail!(
                "CALLS_DEFAULT_TTL_DAYS ({default_ttl_days}) can't exceed \
                 CALLS_MAX_TTL_DAYS ({max_ttl_days})"
            );
        }
        Ok(Self {
            enabled,
            max_ttl_secs: days(max_ttl_days),
            default_ttl_secs: days(default_ttl_days),
            max_active_per_agent: i64::from(number(
                &get,
                "CALLS_MAX_ACTIVE_PER_AGENT",
                DEFAULT_MAX_ACTIVE_PER_AGENT as u32,
                1,
                10_000,
            )?),
            max_messages: number(
                &get,
                "CALLS_MAX_MESSAGES",
                DEFAULT_MAX_MESSAGES as u32,
                1,
                1_000_000,
            )? as i32,
        })
    }
}

fn days(n: u32) -> i64 {
    i64::from(n) * 86_400
}

fn number(
    get: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: u32,
    min: u32,
    max: u32,
) -> anyhow::Result<u32> {
    let Some(raw) = env_value(get, key) else {
        return Ok(default);
    };
    let n: u32 = raw
        .parse()
        .with_context(|| format!("{key}={raw:?} is not a valid number"))?;
    if !(min..=max).contains(&n) {
        bail!("{key} must be between {min} and {max} (got {n})");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn from(pairs: &[(&str, &str)]) -> anyhow::Result<CallsConfig> {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        CallsConfig::from_sources(|key| env.get(key).cloned())
    }

    #[test]
    fn off_with_the_defaults_when_unset_or_empty() {
        assert_eq!(from(&[]).unwrap(), CallsConfig::default());
        assert_eq!(
            from(&[("CALLS_ENABLED", ""), ("CALLS_MAX_TTL_DAYS", " ")]).unwrap(),
            CallsConfig::default()
        );
        let d = CallsConfig::default();
        assert!(!d.enabled);
        assert_eq!(d.max_ttl_secs, 365 * 86_400);
        assert_eq!(d.default_ttl_secs, 30 * 86_400);
    }

    #[test]
    fn values_are_read_and_checked() {
        let c = from(&[
            ("CALLS_ENABLED", "true"),
            ("CALLS_MAX_TTL_DAYS", "90"),
            ("CALLS_DEFAULT_TTL_DAYS", "0"),
            ("CALLS_MAX_ACTIVE_PER_AGENT", "5"),
            ("CALLS_MAX_MESSAGES", "100"),
        ])
        .unwrap();
        assert!(c.enabled);
        assert_eq!(c.max_ttl_secs, 90 * 86_400);
        assert_eq!(c.default_ttl_secs, 0);
        assert_eq!(c.max_active_per_agent, 5);
        assert_eq!(c.max_messages, 100);

        assert!(from(&[("CALLS_ENABLED", "maybe")]).is_err());
        assert!(from(&[("CALLS_MAX_TTL_DAYS", "0")]).is_err());
        assert!(from(&[("CALLS_MAX_MESSAGES", "-1")]).is_err());
        assert!(from(&[
            ("CALLS_MAX_TTL_DAYS", "10"),
            ("CALLS_DEFAULT_TTL_DAYS", "30")
        ])
        .is_err());
    }
}
