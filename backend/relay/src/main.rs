//! `chakramcp-relay` — the inter-agent relay service.
//!
//! Owns:
//! * agents
//! * agent_capabilities
//! * (Phase 1.5+) friendships, grants, audit log, sync execution
//!
//! Reads users / accounts / memberships from the same Postgres but does
//! not write to those tables — that surface belongs to `chakramcp-app`.
//! Both services validate the same JWTs using the shared `JWT_SECRET`,
//! so a token issued by the app's sign-in flow Just Works here.

use std::env;
use std::net::SocketAddr;

use anyhow::Result;
use axum::routing::{get, patch, post};
use axum::Router;
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use chakramcp_shared::{config::SharedConfig, db, tracing_init};

mod auth;
mod handlers;
mod state;

use state::RelayState;

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

fn router(state: RelayState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // ─── Public ────────────────────────────────────
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        // ─── Agents ────────────────────────────────────
        .route(
            "/v1/agents",
            get(handlers::agents::list_mine).post(handlers::agents::create),
        )
        .route(
            "/v1/agents/{id}",
            get(handlers::agents::get_one)
                .patch(handlers::agents::update)
                .delete(handlers::agents::delete),
        )
        // ─── Capabilities ──────────────────────────────
        .route(
            "/v1/agents/{id}/capabilities",
            get(handlers::capabilities::list).post(handlers::capabilities::create),
        )
        .route(
            "/v1/agents/{id}/capabilities/{cap_id}",
            patch(handlers::capabilities::update).delete(handlers::capabilities::delete),
        )
        // ─── Network discovery ─────────────────────────
        .route("/v1/network/agents", get(handlers::agents::list_network))
        // ─── Friendships ───────────────────────────────
        .route(
            "/v1/friendships",
            get(handlers::friendships::list).post(handlers::friendships::propose),
        )
        .route("/v1/friendships/{id}", get(handlers::friendships::get_one))
        .route("/v1/friendships/{id}/accept", post(handlers::friendships::accept))
        .route("/v1/friendships/{id}/reject", post(handlers::friendships::reject))
        .route("/v1/friendships/{id}/counter", post(handlers::friendships::counter))
        .route("/v1/friendships/{id}/cancel", post(handlers::friendships::cancel))
        // ─── Grants ────────────────────────────────────
        .route(
            "/v1/grants",
            get(handlers::grants::list).post(handlers::grants::create),
        )
        .route("/v1/grants/{id}", get(handlers::grants::get_one))
        .route("/v1/grants/{id}/revoke", post(handlers::grants::revoke))
        // ─── Invoke + inbox + audit log ────────────────
        .route("/v1/invoke", post(handlers::invoke::invoke))
        .route("/v1/inbox", get(handlers::invoke::inbox))
        .route("/v1/invocations", get(handlers::invoke::list))
        .route("/v1/invocations/{id}", get(handlers::invoke::get_one))
        .route(
            "/v1/invocations/{id}/result",
            post(handlers::invoke::report_result),
        )
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
