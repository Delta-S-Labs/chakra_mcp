//! MCP beyond the operator tools in `handlers::mcp` (MCP and discovery spec
//! §6): who a call by name is made as and the agents the relay makes for it
//! (`caller`), and waiting for a result (`wait`).

pub mod caller;
pub mod directory;
pub mod discovery;
pub mod rpc;
pub mod wait;

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request};
use chakramcp_shared::hosting::{env_value, parse_bool};

const DEFAULT_TOOL_WAIT_SECS: u64 = 25;
const MAX_TOOL_WAIT_SECS: u64 = 55;
const DEFAULT_MAX_WAITERS: usize = 200;
const DEFAULT_DIRECTORY_PER_IP_PER_MIN: i32 = 60;

/// MCP settings (spec §9), from the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpConfig {
    /// The public directory and agents' own MCP endpoints (`MCP_AGENTS_ENABLED`).
    pub agents_enabled: bool,
    /// How long an agent endpoint's tool call waits for its result
    /// (`MCP_TOOL_WAIT_SECS`, 0–55).
    pub tool_wait_secs: u64,
    /// Tool calls that may wait at once (`MCP_MAX_WAITERS`); beyond it a call
    /// returns "still running" at once.
    pub max_waiters: usize,
    /// The directory's requests per client IP per minute.
    pub directory_per_ip_per_min: i32,
    /// Take the client IP from the last `X-Forwarded-For` entry, which the
    /// proxy in front appends (`TRUST_FORWARDED_FOR`). Off without a proxy:
    /// the header can be spoofed.
    pub trust_forwarded_for: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            agents_enabled: false,
            tool_wait_secs: DEFAULT_TOOL_WAIT_SECS,
            max_waiters: DEFAULT_MAX_WAITERS,
            directory_per_ip_per_min: DEFAULT_DIRECTORY_PER_IP_PER_MIN,
            trust_forwarded_for: false,
        }
    }
}

impl McpConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok())
    }

    /// Each setting from a non-empty value, else its default. A value that
    /// is set but unparseable or out of range is an error.
    pub fn from_sources(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let flag = |key: &str| -> anyhow::Result<bool> {
            match env_value(&get, key) {
                Some(raw) => parse_bool(key, &raw),
                None => Ok(false),
            }
        };
        let number = |key: &str, default: u64, max: u64| -> anyhow::Result<u64> {
            match env_value(&get, key) {
                None => Ok(default),
                Some(raw) => {
                    let n: u64 = raw.parse().map_err(|_| {
                        anyhow::anyhow!("{key} must be a whole number, got {raw:?}")
                    })?;
                    anyhow::ensure!(n <= max, "{key} must be at most {max}, got {n}");
                    Ok(n)
                }
            }
        };
        Ok(Self {
            agents_enabled: flag("MCP_AGENTS_ENABLED")?,
            tool_wait_secs: number(
                "MCP_TOOL_WAIT_SECS",
                DEFAULT_TOOL_WAIT_SECS,
                MAX_TOOL_WAIT_SECS,
            )?,
            max_waiters: number("MCP_MAX_WAITERS", DEFAULT_MAX_WAITERS as u64, 100_000)? as usize,
            directory_per_ip_per_min: number(
                "MCP_DIRECTORY_PER_IP_PER_MIN",
                DEFAULT_DIRECTORY_PER_IP_PER_MIN as u64,
                100_000,
            )? as i32,
            trust_forwarded_for: flag("TRUST_FORWARDED_FOR")?,
        })
    }
}

/// The client's IP. With `trust_forwarded_for`, the last `X-Forwarded-For`
/// entry: the one the proxy in front appended, which a client can't forge
/// (it can only add entries before it). Otherwise, or without the header,
/// the socket peer. `None` when neither is known (a shared bucket).
pub fn client_ip(req: &Request, trust_forwarded_for: bool) -> Option<IpAddr> {
    if trust_forwarded_for {
        let forwarded = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|ip| ip.trim().parse().ok());
        if forwarded.is_some() {
            return forwarded;
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod config_tests {
    use super::*;

    fn from(pairs: &[(&str, &str)]) -> anyhow::Result<McpConfig> {
        McpConfig::from_sources(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(from(&[]).unwrap(), McpConfig::default());
        let cfg = from(&[
            ("MCP_AGENTS_ENABLED", "true"),
            ("MCP_TOOL_WAIT_SECS", "10"),
            ("MCP_MAX_WAITERS", "5"),
            ("MCP_DIRECTORY_PER_IP_PER_MIN", "30"),
            ("TRUST_FORWARDED_FOR", "true"),
            ("CALLS_ENABLED", "nonsense"),
        ])
        .unwrap();
        assert!(cfg.agents_enabled && cfg.trust_forwarded_for);
        assert_eq!(
            (
                cfg.tool_wait_secs,
                cfg.max_waiters,
                cfg.directory_per_ip_per_min
            ),
            (10, 5, 30)
        );
        assert_eq!(
            from(&[("MCP_TOOL_WAIT_SECS", "")]).unwrap().tool_wait_secs,
            25
        );
    }

    #[test]
    fn bad_values_stop_startup() {
        assert!(from(&[("MCP_TOOL_WAIT_SECS", "56")]).is_err());
        assert!(from(&[("MCP_MAX_WAITERS", "-1")]).is_err());
        assert!(from(&[("MCP_AGENTS_ENABLED", "maybe")]).is_err());
    }
}
