//! `chakramcp-server` — orchestrator binary that runs the user-facing
//! API (chakramcp-app) and the inter-agent relay (chakramcp-relay) in
//! one tokio runtime, sharing a Postgres pool and a JWT secret. Aimed
//! at users who want to host a private ChakraMCP network on their own
//! machine via `brew install chakramcp-server`.
//!
//! Subcommands:
//!
//! - `init`    — write a sensible default config to ~/.chakramcp/server.toml
//!   (generates a fresh JWT_SECRET).
//! - `migrate` — apply pending migrations against DATABASE_URL and exit.
//! - `start`   — run app on $APP_PORT (default 8080) and relay on
//!   $RELAY_PORT (default 8090). Migrations are applied
//!   automatically on startup.

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
use chakramcp_shared::{config::SharedConfig, db, telemetry};

#[derive(Parser, Debug)]
#[command(
    name = "chakramcp-server",
    version = telemetry::VERSION,
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

    let database_url = database_url.unwrap_or_else(|| "postgres:///chakramcp".to_string());

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
         log_filter = \"info,chakramcp_app=debug,chakramcp_relay=debug,sqlx=warn\"\n\
         # Log line format: text (default) or json.\n\
         # log_format = \"json\"\n\
         \n\
         # Serve Prometheus metrics at http://<addr>/metrics (off when unset).\n\
         # Keep it private: bind to localhost or an internal network.\n\
         # metrics_addr = \"127.0.0.1:9464\"\n",
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
    telemetry::init_tracing(&cfg.shared.log_filter, cfg.log_format.as_deref());
    let pool = db::connect(&cfg.shared.database_url).await?;
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    eprintln!(
        "migrations applied to {}",
        redact_url(&cfg.shared.database_url)
    );
    Ok(())
}

// ─── start ───────────────────────────────────────────────

async fn start(explicit_path: Option<PathBuf>) -> Result<()> {
    let cfg = load_config(explicit_path)?;
    telemetry::init_tracing(&cfg.shared.log_filter, cfg.log_format.as_deref());
    // Metrics are opt-in (METRICS_ADDR): nothing listens unless it's set.
    if let Some(addr) = cfg.metrics_addr {
        telemetry::install_metrics(addr, telemetry::BuildInfo::CURRENT).await?;
    }

    let pool: PgPool = db::connect(&cfg.shared.database_url).await?;
    sqlx::migrate!("../migrations").run(&pool).await?;

    // Spawn the Agent Card refresh job before mounting the routers
    // so it picks up any push-mode rows immediately on startup. Only
    // active when DISCOVERY_V2 is on; otherwise the job idles and
    // wastes a connection.
    let refresh_shutdown = if cfg.shared.discovery_v2_enabled {
        use chakramcp_relay::agent_card::refresh_job::{
            spawn as spawn_refresh_job, DEFAULT_STALENESS_SECONDS, DEFAULT_TICK_INTERVAL_SECONDS,
        };
        tracing::info!("DISCOVERY_V2 enabled — spawning Agent Card refresh job (server mode)");
        Some(spawn_refresh_job(
            pool.clone(),
            cfg.shared.relay_base_url.clone(),
            DEFAULT_TICK_INTERVAL_SECONDS,
            DEFAULT_STALENESS_SECONDS,
        ))
    } else {
        None
    };

    // Credits: validated settings (bad values stop startup), one refresh of
    // the switches before serving so a deploy never lets an out-of-credits
    // account through, then the worker that keeps them current.
    let credits = chakramcp_relay::limits::CreditsConfig::from_env()?;
    let credit_cache = std::sync::Arc::new(chakramcp_relay::limits::CreditCache::new(
        credits.stale_after(),
    ));
    if let Err(e) = credit_cache
        .refresh(&pool, credits.cost_per_invocation_mc)
        .await
    {
        tracing::warn!(error = %e, "initial credit refresh failed; the worker will retry");
    }
    let worker_pool =
        chakramcp_relay::limits::credits::spawn_worker(&pool, credit_cache.clone(), credits);
    if cfg.metrics_addr.is_some() {
        telemetry::spawn_sampler(vec![
            ("main", pool.clone()),
            ("credits_worker", worker_pool),
        ]);
    }

    let app_state = AppState::new(pool.clone(), cfg.shared.clone())
        .with_upsert_secret(std::env::var("UPSERT_SHARED_SECRET").ok())
        .with_credits_config(credits);
    if app_state.upsert_secret.is_none() {
        tracing::warn!("UPSERT_SHARED_SECRET is not set: Google/GitHub sign-in is disabled");
    }
    let relay_state = RelayState::new(pool, cfg.shared.clone())
        .with_rate_limiter(chakramcp_relay::limits::RateLimiter::from_redis_url(
            std::env::var("REDIS_URL").ok().as_deref(),
        ))
        .with_limits_enforce(chakramcp_relay::limits::enforce_flag(
            std::env::var("LIMITS_ENFORCE").ok().as_deref(),
        ))
        .with_credits_config(credits)
        .with_credit_cache(credit_cache)
        .with_compliance(chakramcp_relay::compliance::ComplianceChecker::from_env());
    // Usage metering runs on a background writer so requests never wait on it.
    let usage = chakramcp_relay::events::UsageRecorder::spawn(relay_state.clone());
    let relay_state = relay_state.with_usage_recorder(usage);

    let app = app_router(app_state);
    let relay = relay_router(relay_state);

    let app_addr = SocketAddr::from(([0, 0, 0, 0], cfg.app_port));
    let relay_addr = SocketAddr::from(([0, 0, 0, 0], cfg.relay_port));

    tracing::info!(%app_addr, %relay_addr, "chakramcp-server starting");

    let app_listener = tokio::net::TcpListener::bind(app_addr).await?;
    let relay_listener = tokio::net::TcpListener::bind(relay_addr).await?;

    // On a shutdown signal both servers stop accepting connections and
    // finish the requests in flight, for at most SHUTDOWN_DRAIN.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let stopped = |mut rx: tokio::sync::watch::Receiver<bool>| async move {
        let _ = rx.changed().await;
    };
    let mut app_handle = tokio::spawn({
        let stop = stopped(stop_rx.clone());
        async move {
            if let Err(err) = axum::serve(app_listener, app)
                .with_graceful_shutdown(stop)
                .await
            {
                tracing::error!(?err, "app server exited with error");
            }
        }
    });
    let mut relay_handle = tokio::spawn({
        let stop = stopped(stop_rx);
        async move {
            if let Err(err) = axum::serve(relay_listener, relay)
                .with_graceful_shutdown(stop)
                .await
            {
                tracing::error!(?err, "relay server exited with error");
            }
        }
    });

    let signalled = tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received — finishing in-flight requests");
            true
        }
        _ = &mut app_handle => {
            tracing::warn!("app server stopped — initiating shutdown");
            false
        }
        _ = &mut relay_handle => {
            tracing::warn!("relay server stopped — initiating shutdown");
            false
        }
    };
    let _ = stop_tx.send(true);
    if signalled {
        let drained = tokio::time::timeout(SHUTDOWN_DRAIN, async {
            let _ = tokio::join!(app_handle, relay_handle);
        })
        .await;
        if drained.is_err() {
            tracing::warn!(
                drain_seconds = SHUTDOWN_DRAIN.as_secs(),
                "requests still open after the drain period — exiting anyway"
            );
        }
    }
    // Stop the refresh loop cleanly so its current tick (if any)
    // can finish before the DB pool drops.
    if let Some(tx) = refresh_shutdown {
        let _ = tx.send(true);
    }
    Ok(())
}

/// How long a shutdown waits for in-flight requests: under Docker's default
/// 10-second stop timeout (Kubernetes allows 30).
const SHUTDOWN_DRAIN: std::time::Duration = std::time::Duration::from_secs(8);

/// Resolves on SIGTERM, which `docker stop` and Kubernetes send, or on
/// Ctrl-C. As a container's PID 1 the server gets no default SIGTERM
/// handling, so without this it would only stop when killed.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "can't listen for SIGTERM; only Ctrl-C stops the server"
                );
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

// ─── Config loading ──────────────────────────────────────

#[derive(Debug, Clone)]
struct ServerConfig {
    shared: SharedConfig,
    app_port: u16,
    relay_port: u16,
    /// Where to serve `/metrics`; `None` = no metrics listener.
    metrics_addr: Option<SocketAddr>,
    /// `text` (default) or `json`; see `telemetry::LogFormat`.
    log_format: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
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
    metrics_addr: Option<String>,
    log_format: Option<String>,
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
            let raw = fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            toml::from_str::<ServerFile>(&raw).with_context(|| format!("parsing {}", p.display()))
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
        .ok_or_else(|| anyhow!("JWT_SECRET is required — set it in env or in the config file"))?;
    let admin_email = std::env::var("ADMIN_EMAIL")
        .ok()
        .or(from_file.admin_email)
        .filter(|s| !s.trim().is_empty());
    let survey_enabled = std::env::var("SURVEY_ENABLED")
        .ok()
        .map(|s| {
            matches!(
                s.trim().to_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        })
        .or(from_file.survey_enabled)
        .unwrap_or(false);

    // Production compose passes *_PUBLIC_URL; older configs use
    // *_BASE_URL. Accept either to avoid silently defaulting to
    // localhost — which would leak into /.well-known discovery
    // metadata, the OAuth issuer claim, and the device-flow
    // verification_uri.
    let frontend_base_url = std::env::var("FRONTEND_BASE_URL")
        .ok()
        .or_else(|| std::env::var("FRONTEND_PUBLIC_URL").ok())
        // Caddy fronts both the marketing site and the OAuth API on
        // the same hostname today, so APP_PUBLIC_URL is the right
        // last-mile fallback before localhost.
        .or_else(|| std::env::var("APP_PUBLIC_URL").ok())
        .or(from_file.frontend_base_url)
        .unwrap_or_else(|| "http://localhost:3000".into());
    let app_base_url = std::env::var("APP_BASE_URL")
        .ok()
        .or_else(|| std::env::var("APP_PUBLIC_URL").ok())
        .or(from_file.app_base_url)
        .unwrap_or_else(|| "http://localhost:8080".into());
    let relay_base_url = std::env::var("RELAY_BASE_URL")
        .ok()
        .or_else(|| std::env::var("RELAY_PUBLIC_URL").ok())
        .or(from_file.relay_base_url)
        .unwrap_or_else(|| "http://localhost:8090".into());

    let discovery_v2_enabled = std::env::var("DISCOVERY_V2")
        .ok()
        .map(|s| {
            matches!(
                s.trim().to_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        })
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

    let metrics_addr = telemetry::parse_metrics_addr(
        std::env::var("METRICS_ADDR")
            .ok()
            .or(from_file.metrics_addr)
            .as_deref(),
    )?;
    let log_format = std::env::var("LOG_FORMAT").ok().or(from_file.log_format);

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
        metrics_addr,
        log_format,
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
