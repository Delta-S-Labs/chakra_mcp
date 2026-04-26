//! CLI config at ~/.chakramcp/config.toml.
//!
//! Holds an array of named networks (the public hosted relay, a
//! self-hosted private one, local dev, …). One is `active` at a time.
//! Every command operates on the active network unless overridden by
//! `--network <name>` or env vars.
//!
//! Schema sketch:
//!
//!   active = "public"
//!
//!   [[networks]]
//!   name = "public"
//!   app_url = "https://chakramcp.com"
//!   relay_url = "https://relay.chakramcp.com"
//!   oauth_client_id = "mcp_..."
//!   [networks.auth]
//!   access_token = "..."
//!   access_token_expires_at = 1234567890
//!   api_key = "..."
//!
//! A legacy single-network schema (the very first dev build) is
//! migrated transparently on first read into a network named `default`.

use anyhow::{anyhow, bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_NETWORK: &str = "default";

/// The hosted public network — what we ship as the first option in the
/// onboarding wizard.
pub const PUBLIC_APP_URL: &str = "https://chakramcp.com";
pub const PUBLIC_RELAY_URL: &str = "https://relay.chakramcp.com";

/// Local dev defaults — handy for `chakramcp networks add local`.
pub const LOCAL_APP_URL: &str = "http://localhost:8080";
pub const LOCAL_RELAY_URL: &str = "http://localhost:8090";

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct CliConfig {
    /// Name of the currently selected network.
    #[serde(default)]
    pub active: Option<String>,

    /// Configured networks. Order is the order we display in
    /// `chakramcp networks list`.
    #[serde(default, rename = "networks")]
    pub networks: Vec<Network>,

    // ---- Legacy fields kept for one-shot migration ----
    #[serde(default, skip_serializing)]
    pub server: Option<LegacyServer>,
    #[serde(default, skip_serializing)]
    pub auth: Option<AuthConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Network {
    pub name: String,
    pub app_url: String,
    pub relay_url: String,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct AuthConfig {
    /// OAuth-issued access token (JWT).
    pub access_token: Option<String>,
    /// Unix timestamp when the access token expires.
    pub access_token_expires_at: Option<i64>,
    /// API key — `ck_…`. Tolerated alongside access_token; token wins
    /// if not yet expired.
    pub api_key: Option<String>,
}

/// Older schema, kept only so we can migrate it forward.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LegacyServer {
    pub app_url: String,
    pub relay_url: String,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
}

impl CliConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: CliConfig =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

        // One-shot migration: pre-networks schema collapses to one
        // network named "default".
        if cfg.networks.is_empty() {
            if let Some(server) = cfg.server.take() {
                cfg.networks.push(Network {
                    name: DEFAULT_NETWORK.to_string(),
                    app_url: server.app_url,
                    relay_url: server.relay_url,
                    oauth_client_id: server.oauth_client_id,
                    auth: cfg.auth.take().unwrap_or_default(),
                });
                cfg.active = Some(DEFAULT_NETWORK.to_string());
                // Persist the migrated form so the legacy keys vanish.
                cfg.save()?;
            }
        }

        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self)?;
        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn active_network(&self) -> Option<&Network> {
        let name = self.active.as_deref()?;
        self.networks.iter().find(|n| n.name == name)
    }

    pub fn active_network_mut(&mut self) -> Option<&mut Network> {
        let name = self.active.clone()?;
        self.networks.iter_mut().find(|n| n.name == name)
    }

    pub fn network(&self, name: &str) -> Option<&Network> {
        self.networks.iter().find(|n| n.name == name)
    }

    pub fn network_mut(&mut self, name: &str) -> Option<&mut Network> {
        self.networks.iter_mut().find(|n| n.name == name)
    }

    /// Insert a new network, returning an error if the name is taken.
    pub fn add_network(&mut self, n: Network) -> Result<()> {
        if self.network(&n.name).is_some() {
            bail!("a network named '{}' already exists", n.name);
        }
        self.networks.push(n);
        Ok(())
    }

    pub fn remove_network(&mut self, name: &str) -> Result<()> {
        let before = self.networks.len();
        self.networks.retain(|n| n.name != name);
        if self.networks.len() == before {
            bail!("no network named '{}'", name);
        }
        if self.active.as_deref() == Some(name) {
            self.active = self.networks.first().map(|n| n.name.clone());
        }
        Ok(())
    }

    pub fn require_active(&self) -> Result<&Network> {
        self.active_network().ok_or_else(|| {
            anyhow!(
                "no active network — run `chakramcp login` to set one up, \
                 or `chakramcp networks list` to see what's configured"
            )
        })
    }
}

impl Network {
    pub fn bearer(&self) -> Option<String> {
        if let (Some(t), Some(exp)) = (
            self.auth.access_token.as_ref(),
            self.auth.access_token_expires_at,
        ) {
            if exp > now_secs() {
                return Some(t.clone());
            }
        }
        self.auth.api_key.clone()
    }

    pub fn auth_kind(&self) -> Option<&'static str> {
        if self.auth.access_token.is_some()
            && self
                .auth
                .access_token_expires_at
                .map(|e| e > now_secs())
                .unwrap_or(false)
        {
            Some("oauth")
        } else if self.auth.api_key.is_some() {
            Some("api_key")
        } else {
            None
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.bearer().is_some()
    }
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "chakramcp", "chakramcp")
        .context("could not resolve a config directory for this OS")?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
