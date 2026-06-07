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
///
/// `PUBLIC_APP_URL` is the *backend* (app service) URL, not the marketing
/// frontend. The CLI hits it for `/v1/me`, OAuth metadata at
/// `/.well-known/oauth-authorization-server`, `/oauth/{authorize,token,
/// device_authorization,register}` — all served by `chakramcp-app` on the
/// `app.` subdomain. `https://chakramcp.com` is the Next.js marketing
/// site; it has *some* OAuth routes (the consent UI at `/oauth/authorize`)
/// but not the metadata or token endpoints, so pointing the CLI at it
/// causes `chakramcp login` → `404 /.well-known/oauth-authorization-server`.
/// See Hermes' bug report on cli-v0.1.0 for the live failure.
pub const PUBLIC_APP_URL: &str = "https://app.chakramcp.com";
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

/// The hosted public network, seeded by default so a fresh install
/// targets `app.chakramcp.com` out of the box — never localhost. Named
/// "public" to match the onboarding wizard's first option. Self-hosters
/// add their own via `chakramcp networks add` and `networks use`.
pub fn public_network() -> Network {
    Network {
        name: "public".to_string(),
        app_url: PUBLIC_APP_URL.to_string(),
        relay_url: PUBLIC_RELAY_URL.to_string(),
        oauth_client_id: None,
        auth: AuthConfig::default(),
    }
}

impl CliConfig {
    /// A brand-new config with the hosted public network active. This is
    /// what a machine with no config file gets, so `chakramcp login`
    /// goes straight to the hosted relay instead of erroring or
    /// defaulting to a localhost dev network.
    fn seeded_public() -> Self {
        CliConfig {
            active: Some("public".to_string()),
            networks: vec![public_network()],
            server: None,
            auth: None,
        }
    }

    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            // First run on this machine: default to the hosted public
            // network rather than an empty config.
            return Ok(Self::seeded_public());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
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
            } else {
                // Config exists but carries no networks and nothing to
                // migrate (e.g. hand-edited down to empty). Seed the
                // hosted public network so the CLI is usable instead of
                // stranded with no active network.
                cfg.networks.push(public_network());
                cfg.active = Some("public".to_string());
                cfg.save()?;
            }
        }

        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_config_defaults_to_hosted_public_network() {
        // A brand-new machine (no config file) must land on the hosted
        // public network — never localhost, never an empty/stranded
        // config. Regression guard for the "CLI pointed at 127.0.0.1"
        // first-run trap.
        let cfg = CliConfig::seeded_public();
        assert_eq!(cfg.active.as_deref(), Some("public"));
        let net = cfg.active_network().expect("active network present");
        assert_eq!(net.name, "public");
        assert_eq!(net.app_url, PUBLIC_APP_URL);
        assert_eq!(net.relay_url, PUBLIC_RELAY_URL);
        assert!(net.app_url.starts_with("https://"));
        assert!(!net.app_url.contains("localhost") && !net.app_url.contains("127.0.0.1"));
    }

    #[test]
    fn public_network_helper_points_hosted() {
        let n = public_network();
        assert_eq!(n.relay_url, "https://relay.chakramcp.com");
        assert_eq!(n.app_url, "https://app.chakramcp.com");
    }
}
