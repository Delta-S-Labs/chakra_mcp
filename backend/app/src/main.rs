//! `chakramcp-app` — the user-facing API service.
//!
//! Owns:
//! * users
//! * accounts (organizations + personal)
//! * account_memberships
//! * oauth_links
//! * api_keys
//! * admin endpoints
//!
//! The relay service (`chakramcp-relay`, sibling crate) reads from
//! these tables but does not write to them. Both validate the same JWTs
//! using the shared `JWT_SECRET`.

use std::env;
use std::net::SocketAddr;

use anyhow::Result;
use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use chakramcp_shared::{config::SharedConfig, db, tracing_init};

mod auth;
mod handlers;
mod state;

use state::AppState;

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

fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // ─── Public ────────────────────────────────────
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        // ─── Sign-in callback from frontend ────────────
        .route("/v1/users/upsert", post(handlers::users::upsert))
        // ─── Authenticated user routes ─────────────────
        .route("/v1/me", get(handlers::users::me))
        .route("/v1/orgs", get(handlers::orgs::list).post(handlers::orgs::create))
        .route("/v1/orgs/{slug}", get(handlers::orgs::get_one))
        .route("/v1/orgs/{slug}/members", get(handlers::orgs::list_members))
        .route("/v1/orgs/{slug}/invites", post(handlers::orgs::create_invite))
        .route(
            "/v1/api-keys",
            get(handlers::api_keys::list).post(handlers::api_keys::create),
        )
        .route("/v1/api-keys/{id}", delete(handlers::api_keys::revoke))
        // ─── Admin ─────────────────────────────────────
        .route("/v1/admin/users", get(handlers::admin::list_users))
        .route("/v1/admin/orgs", get(handlers::admin::list_orgs))
        .route("/v1/admin/api-keys", get(handlers::admin::list_api_keys))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
