//! `chakramcp-server` — orchestrator binary that runs the user-facing
//! API (chakramcp-app) and the inter-agent relay (chakramcp-relay) in
//! one tokio runtime, sharing a Postgres pool and a JWT secret. Aimed
//! at users who want to host a private ChakraMCP network on their own
//! machine via `brew install chakramcp-server`.
//!
//! Subcommands:
//!   * init    — write a sensible default config to ~/.chakramcp/server.toml
//!               (generates a fresh JWT_SECRET).
//!   * migrate — apply pending migrations against DATABASE_URL and exit.
//!   * start   — run app on $APP_PORT (default 8080) and relay on
//!               $RELAY_PORT (default 8090). Migrations are applied
//!               automatically on startup.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use rand::RngCore;
use sqlx::PgPool;
use tokio::signal;

use chakramcp_app::{router as app_router, AppState};
use chakramcp_relay::{router as relay_router, RelayState};
use chakramcp_shared::{config::SharedConfig, db, tracing_init};

#[derive(Parser, Debug)]
#[command(
    name = "chakramcp-server",
    version,
    about = "Run a private ChakraMCP network locally.",
    long_about = "Runs the user-facing API + inter-agent relay services in one process. \
                  Pair with a Postgres instance (homebrew installs postgresql@16 alongside)."
)]
struct Cli {
    /// Path to the server config file (TOML). Defaults to
    /// ~/.chakramcp/server.toml.
    #[arg(long, env = "CHAKRAMCP_SERVER_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Write a default server config + generate a fresh JWT secret.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
        /// Postgres connection string. Default points at the homebrew
        /// postgresql@16 socket on macOS.
        #[arg(long)]
        database_url: Option<String>,
        /// Email of the bootstrap admin user (matches ADMIN_EMAIL in
        /// the env-var path).
        #[arg(long)]
        admin_email: Option<String>,
    },
    /// Apply pending migrations and exit.
    Migrate,
    /// Run app + relay together (default if no subcommand is given).
    Start,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cmd = cli.cmd.unwrap_or(Cmd::Start);
    match cmd {
        Cmd::Init {
            force,
            database_url,
            admin_email,
        } => init(cli.config, force, database_url, admin_email),
        Cmd::Migrate => migrate(cli.config).await,
        Cmd::Start => start(cli.config).await,
    }
}

// ─── init ────────────────────────────────────────────────

fn init(
    explicit_path: Option<PathBuf>,
    force: bool,
    database_url: Option<String>,
    admin_email: Option<String>,
) -> Result<()> {
    let path = explicit_path.unwrap_or(default_config_path()?);
    if path.exists() && !force {
        return Err(anyhow!(
            "{} already exists — pass --force to overwrite",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut secret_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret_bytes);
    let jwt_secret = hex::encode(secret_bytes);

    let database_url =
        database_url.unwrap_or_else(|| "postgres:///chakramcp".to_string());

    let admin_line = match admin_email.as_deref() {
        Some(e) if !e.is_empty() => format!("admin_email = \"{e}\"\n"),
        _ => "# admin_email = \"you@example.com\"\n".to_string(),
    };

    let body = format!(
        "# chakramcp-server config — created by `chakramcp-server init`.\n\
         # Edit and re-run `chakramcp-server start`.\n\
         \n\
         database_url = \"{database_url}\"\n\
         jwt_secret = \"{jwt_secret}\"\n\
         {admin_line}\
         # survey_enabled = false\n\
         \n\
         # Public-facing URLs — used by the OAuth discovery doc the\n\
         # MCP server points clients at. Defaults assume you're running\n\
         # locally; change to https://your.host when you put a TLS\n\
         # terminator in front.\n\
         frontend_base_url = \"http://localhost:3000\"\n\
         app_base_url = \"http://localhost:8080\"\n\
         relay_base_url = \"http://localhost:8090\"\n\
         \n\
         # Listening ports.\n\
         app_port = 8080\n\
         relay_port = 8090\n\
         \n\
         # Logging filter (RUST_LOG syntax).\n\
         log_filter = \"info,chakramcp_app=debug,chakramcp_relay=debug,sqlx=warn\"\n",
    );
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    eprintln!("wrote {}", path.display());
    eprintln!("next: chakramcp-server migrate && chakramcp-server start");
    Ok(())
}

// ─── migrate ─────────────────────────────────────────────

async fn migrate(explicit_path: Option<PathBuf>) -> Result<()> {
    let cfg = load_config(explicit_path)?;
    tracing_init::init(&cfg.shared.log_filter);
    let pool = db::connect(&cfg.shared.database_url).await?;
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    eprintln!("migrations applied to {}", redact_url(&cfg.shared.database_url));
    Ok(())
}

// ─── start ───────────────────────────────────────────────

async fn start(explicit_path: Option<PathBuf>) -> Result<()> {
    let cfg = load_config(explicit_path)?;
    tracing_init::init(&cfg.shared.log_filter);

    let pool: PgPool = db::connect(&cfg.shared.database_url).await?;
    sqlx::migrate!("../migrations").run(&pool).await?;

    let app_state = AppState::new(pool.clone(), cfg.shared.clone());
    let relay_state = RelayState::new(pool, cfg.shared.clone());

    let app = app_router(app_state);
    let relay = relay_router(relay_state);

    let app_addr = SocketAddr::from(([0, 0, 0, 0], cfg.app_port));
    let relay_addr = SocketAddr::from(([0, 0, 0, 0], cfg.relay_port));

    tracing::info!(%app_addr, %relay_addr, "chakramcp-server starting");

    let app_listener = tokio::net::TcpListener::bind(app_addr).await?;
    let relay_listener = tokio::net::TcpListener::bind(relay_addr).await?;

    let app_handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(app_listener, app).await {
            tracing::error!(?err, "app server exited with error");
        }
    });
    let relay_handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(relay_listener, relay).await {
            tracing::error!(?err, "relay server exited with error");
        }
    });

    tokio::select! {
        _ = signal::ctrl_c() => {
            tracing::info!("ctrl-c received — shutting down");
        }
        _ = app_handle => {
            tracing::warn!("app server stopped — initiating shutdown");
        }
        _ = relay_handle => {
            tracing::warn!("relay server stopped — initiating shutdown");
        }
    }
    Ok(())
}

// ─── Config loading ──────────────────────────────────────

#[derive(Debug, Clone)]
struct ServerConfig {
    shared: SharedConfig,
    app_port: u16,
    relay_port: u16,
}

#[derive(Debug, serde::Deserialize)]
struct ServerFile {
    database_url: Option<String>,
    jwt_secret: Option<String>,
    admin_email: Option<String>,
    survey_enabled: Option<bool>,
    frontend_base_url: Option<String>,
    app_base_url: Option<String>,
    relay_base_url: Option<String>,
    discovery_v2_enabled: Option<bool>,
    app_port: Option<u16>,
    relay_port: Option<u16>,
    log_filter: Option<String>,
}

fn load_config(explicit_path: Option<PathBuf>) -> Result<ServerConfig> {
    // Precedence: --config / CHAKRAMCP_SERVER_CONFIG → env-var fallback
    // (so the existing chakramcp-app deploy story still works
    // without a config file).
    let path = explicit_path.or_else(|| default_config_path().ok());

    let from_file = path
        .as_ref()
        .filter(|p| p.exists())
        .map(|p| {
            let raw = fs::read_to_string(p)
                .with_context(|| format!("reading {}", p.display()))?;
            toml::from_str::<ServerFile>(&raw)
                .with_context(|| format!("parsing {}", p.display()))
        })
        .transpose()?
        .unwrap_or_default_marker();

    // Env wins over file for individual fields, so production deploys
    // can override anything via the orchestration layer.
    let database_url = std::env::var("DATABASE_URL")
        .ok()
        .or(from_file.database_url)
        .ok_or_else(|| {
            anyhow!(
                "DATABASE_URL is required — set it in env or in the config file \
                 (run `chakramcp-server init` to create one)"
            )
        })?;
    let jwt_secret = std::env::var("JWT_SECRET")
        .ok()
        .or(from_file.jwt_secret)
        .ok_or_else(|| {
            anyhow!(
                "JWT_SECRET is required — set it in env or in the config file"
            )
        })?;
    let admin_email = std::env::var("ADMIN_EMAIL")
        .ok()
        .or(from_file.admin_email)
        .filter(|s| !s.trim().is_empty());
    let survey_enabled = std::env::var("SURVEY_ENABLED")
        .ok()
        .map(|s| matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .or(from_file.survey_enabled)
        .unwrap_or(false);

    let frontend_base_url = std::env::var("FRONTEND_BASE_URL")
        .ok()
        .or(from_file.frontend_base_url)
        .unwrap_or_else(|| "http://localhost:3000".into());
    let app_base_url = std::env::var("APP_BASE_URL")
        .ok()
        .or(from_file.app_base_url)
        .unwrap_or_else(|| "http://localhost:8080".into());
    let relay_base_url = std::env::var("RELAY_BASE_URL")
        .ok()
        .or(from_file.relay_base_url)
        .unwrap_or_else(|| "http://localhost:8090".into());

    let discovery_v2_enabled = std::env::var("DISCOVERY_V2")
        .ok()
        .map(|s| matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .or(from_file.discovery_v2_enabled)
        .unwrap_or(false);

    let log_filter = std::env::var("RUST_LOG")
        .ok()
        .or(from_file.log_filter)
        .unwrap_or_else(|| "info,chakramcp_app=debug,chakramcp_relay=debug,sqlx=warn".into());

    let app_port = std::env::var("APP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .or(from_file.app_port)
        .unwrap_or(8080);
    let relay_port = std::env::var("RELAY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .or(from_file.relay_port)
        .unwrap_or(8090);

    Ok(ServerConfig {
        shared: SharedConfig {
            database_url,
            jwt_secret,
            admin_email,
            survey_enabled,
            frontend_base_url,
            app_base_url,
            relay_base_url,
            discovery_v2_enabled,
            log_filter,
        },
        app_port,
        relay_port,
    })
}

fn default_config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "chakramcp", "chakramcp")
        .ok_or_else(|| anyhow!("could not resolve a config directory for this OS"))?;
    Ok(dirs.config_dir().join("server.toml"))
}

fn redact_url(url: &str) -> String {
    // Strip the password from a postgres:// URL for log readability.
    match url::Url::parse(url) {
        Ok(mut u) => {
            let _ = u.set_password(None);
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

// `Default` for ServerFile so the missing-file path returns an
// all-None struct without us writing it out by hand.
impl Default for ServerFile {
    fn default() -> Self {
        Self {
            database_url: None,
            jwt_secret: None,
            admin_email: None,
            survey_enabled: None,
            frontend_base_url: None,
            app_base_url: None,
            relay_base_url: None,
            discovery_v2_enabled: None,
            app_port: None,
            relay_port: None,
            log_filter: None,
        }
    }
}

trait UnwrapOrDefaultMarker {
    type Inner;
    fn unwrap_or_default_marker(self) -> Self::Inner;
}
impl UnwrapOrDefaultMarker for Option<ServerFile> {
    type Inner = ServerFile;
    fn unwrap_or_default_marker(self) -> ServerFile {
        self.unwrap_or_default()
    }
}
