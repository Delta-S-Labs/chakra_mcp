//! Standalone entry point for `chakramcp-app`. The library lives in
//! lib.rs so the orchestrator binary (`chakramcp-server`) can mount
//! the same router in-process.

use std::env;
use std::net::SocketAddr;

use anyhow::Result;
use sqlx::PgPool;

use chakramcp_app::{router, AppState};
use chakramcp_shared::{config::SharedConfig, db, tracing_init};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = SharedConfig::from_env()?;
    tracing_init::init(&cfg.log_filter);

    let pool: PgPool = db::connect(&cfg.database_url).await?;
    sqlx::migrate!("../migrations").run(&pool).await?;

    let state = AppState::new(pool, cfg.clone());
    let app = router(state);

    let port: u16 = env::var("APP_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "chakramcp-app starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
