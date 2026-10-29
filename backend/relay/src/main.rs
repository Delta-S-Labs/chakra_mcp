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
use chakramcp_shared::{config::SharedConfig, db, telemetry};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = SharedConfig::from_env()?;
    telemetry::init_tracing(&cfg.log_filter, env::var("LOG_FORMAT").ok().as_deref());
    // Metrics are opt-in (METRICS_ADDR): nothing listens unless it's set.
    let metrics_addr = telemetry::parse_metrics_addr(env::var("METRICS_ADDR").ok().as_deref())?;
    if let Some(addr) = metrics_addr {
        telemetry::install_metrics(addr, telemetry::BuildInfo::CURRENT).await?;
    }

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
    let hosting = chakramcp_shared::hosting::HostingSettings::from_env()?;
    tracing::info!("{}", hosting.summary());
    // Which addresses requests to agent-supplied URLs may reach.
    let outbound = chakramcp_relay::outbound::OutboundPolicy::from_env(hosting.mode)?;
    tracing::info!("{}", outbound.summary());

    let refresh_shutdown = if cfg.discovery_v2_enabled {
        tracing::info!("DISCOVERY_V2 enabled — spawning Agent Card refresh job");
        Some(spawn_refresh_job(
            pool.clone(),
            cfg.relay_base_url.clone(),
            DEFAULT_TICK_INTERVAL_SECONDS,
            DEFAULT_STALENESS_SECONDS,
            outbound,
        ))
    } else {
        None
    };

    // Credits: validated settings (bad values stop startup), one refresh of
    // the switches before serving so a deploy never lets an out-of-credits
    // account through, then the worker that keeps them current.
    let credits = chakramcp_relay::limits::CreditsConfig::from_env(hosting.credits_enabled)?;
    let credit_cache = std::sync::Arc::new(chakramcp_relay::limits::CreditCache::new(
        credits.stale_after(),
    ));
    if let Err(e) = credit_cache.refresh(&pool, &credits).await {
        tracing::warn!(error = %e, "initial credit refresh failed; the worker will retry");
    }
    let worker_pool =
        chakramcp_relay::limits::credits::spawn_worker(&pool, credit_cache.clone(), credits);
    if metrics_addr.is_some() {
        telemetry::spawn_sampler(vec![
            ("main", pool.clone()),
            ("credits_worker", worker_pool),
        ]);
    }

    // Usage events older than this are deleted (USAGE_RETENTION_DAYS, default
    // 90 days; 0 keeps them). A bad value stops startup.
    let usage_retention =
        chakramcp_relay::retention::usage_retention_days(|key| env::var(key).ok())?;
    let calls = chakramcp_relay::calls::CallsConfig::from_env()?;
    let wait = chakramcp_shared::wake::WakeSettings::from_env()?;
    tracing::info!("{}", wait.summary());
    let wake = chakramcp_shared::wake::Wake::new(wait.max_waiters);
    chakramcp_shared::wake::bus::spawn(pool.clone(), wake.clone());
    let state = RelayState::new(pool, cfg.clone())
        .with_rate_limiter(chakramcp_relay::limits::RateLimiter::from_redis_url(
            env::var("REDIS_URL").ok().as_deref(),
        ))
        .with_limits_enforce(chakramcp_relay::limits::enforce_flag(
            env::var("LIMITS_ENFORCE").ok().as_deref(),
        ))
        .with_credits_config(credits)
        .with_credits_url(chakramcp_relay::limits::credits_url(
            hosting.is_managed(),
            &cfg.frontend_base_url,
        ))
        .with_credit_cache(credit_cache)
        .with_featured_account(env::var("DISCOVERY_FEATURED_ACCOUNT").ok())
        .with_usage_retention_days(usage_retention)
        .with_compliance(chakramcp_relay::compliance::ComplianceChecker::from_env())
        .with_calls(calls)
        .with_memory(chakramcp_relay::calls::memory::client(&calls))
        .with_mcp(chakramcp_relay::mcp::McpConfig::from_env()?)
        .with_outbound(outbound)
        .with_wake(wake.clone(), wait);
    state.heartbeats.clone().spawn_flusher(state.db.clone());
    if let Some(days) = usage_retention {
        chakramcp_relay::retention::spawn_usage_pruner(state.db.clone(), days);
    }
    // Owner consent: expire undecided requests and prune old notices.
    chakramcp_relay::consent::expiry::spawn_expiry_worker(state.db.clone(), state.wake.clone());
    if state.calls.enabled {
        tracing::info!("calls enabled");
        chakramcp_relay::calls::workers::spawn_workers(state.db.clone(), state.wake.clone());
        if state.memory.is_some() {
            tracing::info!("calls memory enabled");
            chakramcp_relay::calls::memory::tagger::spawn_tagger(state.clone());
        }
    }
    // Usage metering runs on a background writer so requests never wait on it.
    let usage = chakramcp_relay::events::UsageRecorder::spawn(state.clone());
    let state = state.with_usage_recorder(usage);
    let app = router(state);

    let port: u16 = env::var("RELAY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8090);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "chakramcp-relay starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // The socket peer is the client's IP for the public directory's per-IP
    // limit when no trusted proxy says otherwise.
    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        // Waiting inbox pulls return now instead of holding the shutdown.
        wake.close();
    })
    .await;

    // On shutdown signal the refresh loop to exit cleanly so its
    // current tick (if any) finishes before DB pool closes.
    if let Some(tx) = refresh_shutdown {
        let _ = tx.send(true);
    }
    serve_result?;
    Ok(())
}
