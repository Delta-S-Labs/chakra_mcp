//! `chakramcp-relay` — inter-agent relay service, also reusable as a
//! library so the supervisor binary (`chakramcp-server`) can mount its
//! router into the same process as the app.

use axum::routing::{get, patch, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub mod agent_card;
pub mod auth;
pub mod forwarder;
pub mod handlers;
pub mod inbox_bridge;
pub mod jwt_mint;
pub mod policy;
pub mod state;

pub use state::RelayState;

pub fn router(state: RelayState) -> Router {
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
        // ─── MCP server ────────────────────────────────
        .route(
            "/.well-known/oauth-protected-resource",
            get(handlers::mcp::protected_resource_metadata),
        )
        .route("/mcp", post(handlers::mcp::handle))
        // ─── A2A: JWKS for verifying our Agent Card signatures ─
        .route("/.well-known/jwks.json", get(handlers::jwks::get_jwks))
        // ─── A2A: published Agent Card per registered agent ────
        .route(
            "/agents/{account_slug}/{agent_slug}/.well-known/agent-card.json",
            get(handlers::published_cards::get_agent_card),
        )
        // ─── A2A: JSON-RPC + streaming endpoints (stubs until D5) ─
        .route(
            "/agents/{account_slug}/{agent_slug}/a2a/jsonrpc",
            post(handlers::a2a::jsonrpc_stub),
        )
        .route(
            "/agents/{account_slug}/{agent_slug}/a2a/stream",
            post(handlers::a2a::stream_stub),
        )
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
