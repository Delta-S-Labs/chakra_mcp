use std::env;

use anyhow::{Context, Result};

/// Configuration shared by all backend services. Each service may add its
/// own struct on top of this for service-specific knobs.
#[derive(Debug, Clone)]
pub struct SharedConfig {
    pub database_url: String,
    pub jwt_secret: String,
    pub admin_email: Option<String>,
    pub log_filter: String,
}

impl SharedConfig {
    pub fn from_env() -> Result<Self> {
        // Best-effort .env loading. The repo root .env.local takes precedence.
        let _ = dotenvy::from_filename(".env.local");
        let _ = dotenvy::dotenv();

        let database_url = env::var("DATABASE_URL")
            .context("DATABASE_URL is required (e.g. postgres://chakramcp:chakramcp@localhost:5432/chakramcp)")?;

        let jwt_secret = env::var("JWT_SECRET")
            .context("JWT_SECRET is required (generate with: openssl rand -hex 32)")?;

        let admin_email = env::var("ADMIN_EMAIL").ok().filter(|s| !s.trim().is_empty());

        let log_filter = env::var("RUST_LOG")
            .unwrap_or_else(|_| "info,chakramcp_app=debug,chakramcp_relay=debug,sqlx=warn".to_string());

        Ok(Self {
            database_url,
            jwt_secret,
            admin_email,
            log_filter,
        })
    }
}
