//! Standalone entry point for `chakramcp-app`. The library lives in
//! lib.rs so the orchestrator binary (`chakramcp-server`) can mount
//! the same router in-process.

use std::env;
use std::net::SocketAddr;

use anyhow::Result;
use sqlx::PgPool;

use chakramcp_app::{router, AppState};
use chakramcp_shared::{config::SharedConfig, db, telemetry};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = SharedConfig::from_env()?;
    telemetry::init_tracing(&cfg.log_filter, env::var("LOG_FORMAT").ok().as_deref());
    // Metrics are opt-in (METRICS_ADDR): nothing listens unless it's set.
    let metrics_addr = telemetry::parse_metrics_addr(env::var("METRICS_ADDR").ok().as_deref())?;
    if let Some(addr) = metrics_addr {
        let build = telemetry::BuildInfo {
            version: env!("CARGO_PKG_VERSION"),
            git_sha: option_env!("GIT_SHA").unwrap_or("unknown"),
        };
        telemetry::install_metrics(addr, build).await?;
    }

    let pool: PgPool = db::connect(&cfg.database_url).await?;
    if metrics_addr.is_some() {
        telemetry::spawn_sampler(vec![("main", pool.clone())]);
    }
    sqlx::migrate!("../migrations").run(&pool).await?;

    let state =
        AppState::new(pool, cfg.clone()).with_upsert_secret(env::var("UPSERT_SHARED_SECRET").ok());
    let app = router(state);

    let port: u16 = env::var("APP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "chakramcp-app starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
