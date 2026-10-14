use std::env;

use anyhow::{Context, Result};

/// Configuration shared by all backend services. Each service may add its
/// own struct on top of this for service-specific knobs.
#[derive(Debug, Clone)]
pub struct SharedConfig {
    pub database_url: String,
    pub jwt_secret: String,
    pub admin_email: Option<String>,
    /// When true, the network expects every new user to complete the
    /// first-login survey before they can use the app. Defaults to
    /// false (private deployments don't need this); the hosted public
    /// network sets it to true.
    pub survey_enabled: bool,
    /// Public-facing base URL for the user-facing app (where the OAuth
    /// consent page lives). Used to build authorization_endpoint in
    /// the OAuth discovery doc.
    pub frontend_base_url: String,
    /// Public-facing base URL for the chakramcp-app service (token,
    /// register endpoints).
    pub app_base_url: String,
    /// Public-facing base URL for the chakramcp-relay service.
    pub relay_base_url: String,
    /// A2A discovery v2 feature flag (the migration described in
    /// `docs/specs/2026-04-29-discovery-implementation-plan.md`).
    /// When false, new A2A surfaces (Agent Card serve, JWKS, A2A
    /// JSON-RPC routes) return 404. Defaults to false until the
    /// cutover phase D14.
    pub discovery_v2_enabled: bool,
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

        let admin_email = env::var("ADMIN_EMAIL")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let survey_enabled = env::var("SURVEY_ENABLED")
            .ok()
            .map(|s| {
                matches!(
                    s.trim().to_lowercase().as_str(),
                    "true" | "1" | "yes" | "on"
                )
            })
            .unwrap_or(false);

        // Production compose passes APP_PUBLIC_URL / RELAY_PUBLIC_URL /
        // FRONTEND_PUBLIC_URL; older configs and the dev workflow use
        // *_BASE_URL. Accept either to avoid a silent fallback to the
        // localhost defaults — which would land in /.well-known
        // discovery metadata, the device_authorization response, and
        // every OAuth issuer claim. Caused a real prod incident.
        let app_base_url = env::var("APP_BASE_URL")
            .or_else(|_| env::var("APP_PUBLIC_URL"))
            .unwrap_or_else(|_| "http://localhost:8080".to_string());
        // Where the sign-in and pairing pages live: the web UI when one is
        // configured, else this server's app URL, which serves its own on a
        // self-hosted server. Local development sets FRONTEND_BASE_URL to the
        // web UI on :3000.
        let frontend_base_url = resolve_frontend_url(|key| env::var(key).ok(), None, &app_base_url);
        let relay_base_url = env::var("RELAY_BASE_URL")
            .or_else(|_| env::var("RELAY_PUBLIC_URL"))
            .unwrap_or_else(|_| "http://localhost:8090".to_string());

        let discovery_v2_enabled = env::var("DISCOVERY_V2")
            .ok()
            .map(|s| {
                matches!(
                    s.trim().to_lowercase().as_str(),
                    "true" | "1" | "yes" | "on"
                )
            })
            .unwrap_or(false);

        let log_filter = env::var("RUST_LOG").unwrap_or_else(|_| {
            "info,chakramcp_app=debug,chakramcp_relay=debug,sqlx=warn".to_string()
        });

        Ok(Self {
            database_url,
            jwt_secret,
            admin_email,
            survey_enabled,
            frontend_base_url,
            app_base_url,
            relay_base_url,
            discovery_v2_enabled,
            log_filter,
        })
    }
}

/// Where the sign-in and pairing pages live, which the OAuth discovery
/// document and the device flow point at: `FRONTEND_BASE_URL`, then
/// `FRONTEND_PUBLIC_URL` (empty values count as unset), then the config
/// file's `frontend_base_url`, else the app URL itself. A self-hosted
/// server serves its own pages there; chakramcp.com and local development
/// point at the web UI.
pub fn resolve_frontend_url(
    env: impl Fn(&str) -> Option<String>,
    from_file: Option<String>,
    app_base_url: &str,
) -> String {
    let non_empty = |key: &str| env(key).filter(|v| !v.trim().is_empty());
    non_empty("FRONTEND_BASE_URL")
        .or_else(|| non_empty("FRONTEND_PUBLIC_URL"))
        .or(from_file)
        .unwrap_or_else(|| app_base_url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::resolve_frontend_url;

    const APP: &str = "https://app.example.com";

    fn resolve(env: &[(&str, &str)], from_file: Option<&str>) -> String {
        resolve_frontend_url(
            |key| {
                env.iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
            },
            from_file.map(str::to_owned),
            APP,
        )
    }

    #[test]
    fn with_nothing_set_the_app_serves_its_own_pages() {
        assert_eq!(resolve(&[], None), APP);
        assert_eq!(
            resolve(
                &[("FRONTEND_PUBLIC_URL", " "), ("FRONTEND_BASE_URL", "")],
                None
            ),
            APP,
            "empty values count as unset"
        );
    }

    #[test]
    fn a_configured_web_ui_wins() {
        // chakramcp.com
        assert_eq!(
            resolve(&[("FRONTEND_PUBLIC_URL", "https://chakramcp.com")], None),
            "https://chakramcp.com"
        );
        // Local development: the web UI on :3000.
        assert_eq!(
            resolve(
                &[
                    ("FRONTEND_BASE_URL", "http://localhost:3000"),
                    ("FRONTEND_PUBLIC_URL", "https://ignored.example")
                ],
                None
            ),
            "http://localhost:3000"
        );
    }

    #[test]
    fn the_config_file_comes_after_the_environment() {
        // A 0.2.0 `server.toml` still carries `frontend_base_url =
        // "http://localhost:3000"`: it's kept (the server warns about it).
        assert_eq!(
            resolve(&[], Some("http://localhost:3000")),
            "http://localhost:3000"
        );
        assert_eq!(
            resolve(
                &[("FRONTEND_PUBLIC_URL", "https://ui.example")],
                Some("http://localhost:3000")
            ),
            "https://ui.example"
        );
    }
}
