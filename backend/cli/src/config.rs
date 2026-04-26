//! Persistent CLI config at ~/.chakramcp/config.toml.
//!
//! Two relevant sections:
//!   [server]   — base URLs (app + relay) and the dynamically-registered
//!                OAuth client_id used by `chakramcp login`.
//!   [auth]     — either an OAuth-issued JWT (with expiry) or an API key.
//!
//! Either auth method produces the same Bearer header at the wire.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_APP_URL: &str = "http://localhost:8080";
const DEFAULT_RELAY_URL: &str = "http://localhost:8090";

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct CliConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub app_url: String,
    pub relay_url: String,
    pub oauth_client_id: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            app_url: DEFAULT_APP_URL.into(),
            relay_url: DEFAULT_RELAY_URL.into(),
            oauth_client_id: None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct AuthConfig {
    /// OAuth-issued access token (JWT).
    pub access_token: Option<String>,
    /// Unix timestamp when the access token expires.
    pub access_token_expires_at: Option<i64>,
    /// API key — `ck_…`. Mutually exclusive with access_token in practice
    /// but we tolerate both being present (token wins if not expired).
    pub api_key: Option<String>,
}

impl CliConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self)?;
        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        // Best-effort restrict to user-only (Unix). Windows has its own ACL story we don't touch.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Bearer token to send. OAuth access_token wins if present and unexpired.
    pub fn bearer(&self) -> Option<String> {
        if let (Some(t), Some(exp)) = (
            self.auth.access_token.as_ref(),
            self.auth.access_token_expires_at,
        ) {
            let now = chrono_now();
            if exp > now {
                return Some(t.clone());
            }
        }
        self.auth.api_key.clone()
    }

    pub fn auth_kind(&self) -> Option<&'static str> {
        if self.bearer().is_some() {
            if self.auth.access_token.is_some() {
                Some("oauth")
            } else {
                Some("api_key")
            }
        } else {
            None
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "chakramcp", "chakramcp")
        .context("could not resolve a config directory for this OS")?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn chrono_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
