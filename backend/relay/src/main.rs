//! Standalone entry point for `chakramcp-relay`. The library lives in
//! lib.rs so the orchestrator binary (`chakramcp-server`) can mount
//! the same router in-process.

use std::env;
use std::net::SocketAddr;

use anyhow::Result;
use sqlx::PgPool;

use chakramcp_relay::{router, RelayState};
use chakramcp_shared::{config::SharedConfig, db, tracing_init};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = SharedConfig::from_env()?;
    tracing_init::init(&cfg.log_filter);

    let pool: PgPool = db::connect(&cfg.database_url).await?;
    // The app crate owns migrations — when the two services share a
    // database we trust whichever booted first to migrate. Running the
    // migrator here is still safe (it's idempotent) and means the relay
    // can boot a fresh dev DB on its own.
    sqlx::migrate!("../migrations").run(&pool).await?;

    let state = RelayState::new(pool, cfg.clone());
    let app = router(state);

    let port: u16 = env::var("RELAY_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8090);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "chakramcp-relay starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
