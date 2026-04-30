//! Standalone entry point for `chakramcp-relay`. The library lives in
//! lib.rs so the orchestrator binary (`chakramcp-server`) can mount
//! the same router in-process.

use std::env;
use std::net::SocketAddr;

use anyhow::Result;
use sqlx::PgPool;

use chakramcp_relay::{
    agent_card::refresh_job::{
        spawn as spawn_refresh_job, DEFAULT_STALENESS_SECONDS, DEFAULT_TICK_INTERVAL_SECONDS,
    },
    router, RelayState,
};
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

    // Refresh job for push-mode Agent Cards. Spawn only when the
    // discovery v2 surface is enabled — otherwise it'd hammer
    // upstream URLs that can't be served. SKIP LOCKED makes
    // multi-replica spawn safe.
    let refresh_shutdown = if cfg.discovery_v2_enabled {
        tracing::info!("DISCOVERY_V2 enabled — spawning Agent Card refresh job");
        Some(spawn_refresh_job(
            pool.clone(),
            cfg.relay_base_url.clone(),
            DEFAULT_TICK_INTERVAL_SECONDS,
            DEFAULT_STALENESS_SECONDS,
        ))
    } else {
        None
    };

    let state = RelayState::new(pool, cfg.clone());
    let app = router(state);

    let port: u16 = env::var("RELAY_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8090);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "chakramcp-relay starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let serve_result = axum::serve(listener, app).await;

    // On shutdown signal the refresh loop to exit cleanly so its
    // current tick (if any) finishes before DB pool closes.
    if let Some(tx) = refresh_shutdown {
        let _ = tx.send(true);
    }
    serve_result?;
    Ok(())
}
