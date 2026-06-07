//! `chakramcp-relay` — inter-agent relay service, also reusable as a
//! library so the supervisor binary (`chakramcp-server`) can mount its
//! router into the same process as the app.

use axum::extract::{MatchedPath, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::{from_fn_with_state, Next};
use axum::response::Response;
use axum::routing::{get, patch, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub mod agent_card;
pub mod auth;
pub mod events;
pub mod forwarder;
pub mod handlers;
pub mod inbox_bridge;
pub mod jwt_mint;
pub mod policy;
pub mod state;

pub use state::RelayState;

/// Usage-metering middleware: records one `usage_events` row per REST
/// request (every GET/POST/PATCH/DELETE), attributed to the caller. The
/// `/mcp` endpoint meters per-tool inside its dispatcher instead, and
/// health/well-known probes are skipped as noise.
async fn usage_middleware(State(state): State<RelayState>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());
    let actor = {
        let header = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        auth::authenticate(&state, header).await
    };

    let resp = next.run(req).await;

    let skip = route == "/mcp"
        || route.starts_with("/healthz")
        || route.starts_with("/readyz")
        || route.starts_with("/.well-known");
    if !skip {
        let status = resp.status().as_u16() as i32;
        let action = format!("{method} {route}");
        events::record_usage(
            &state.db,
            actor.as_ref(),
            None,
            "rest",
            &action,
            &method,
            &route,
            status,
        )
        .await;
    }
    resp
}

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
        // ─── Reviews (sub-project 2 of the ratings feature) ────
        .route(
            "/v1/agents/{target_agent_id}/reviews",
            get(handlers::reviews::list).post(handlers::reviews::write),
        )
        .route(
            "/v1/agents/{target_agent_id}/reviews/eligibility",
            get(handlers::reviews::eligibility),
        )
        .route(
            "/v1/agents/{target_agent_id}/reviews/{review_id}/hide",
            post(handlers::reviews::hide),
        )
        .route(
            "/v1/agents/{target_agent_id}/reviews/{review_id}/unhide",
            post(handlers::reviews::unhide),
        )
        // ─── Network discovery ─────────────────────────
        .route("/v1/network/agents", get(handlers::agents::list_network))
        // ─── Friendships ───────────────────────────────
        .route(
            "/v1/friendships",
            get(handlers::friendships::list).post(handlers::friendships::propose),
        )
        .route("/v1/friendships/{id}", get(handlers::friendships::get_one))
        .route(
            "/v1/friendships/{id}/accept",
            post(handlers::friendships::accept),
        )
        .route(
            "/v1/friendships/{id}/reject",
            post(handlers::friendships::reject),
        )
        .route(
            "/v1/friendships/{id}/counter",
            post(handlers::friendships::counter),
        )
        .route(
            "/v1/friendships/{id}/cancel",
            post(handlers::friendships::cancel),
        )
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
        // ─── Discovery search (D10a) ──────────────────────────
        .route("/v1/discovery/agents", get(handlers::discovery::search))
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
        .layer(from_fn_with_state(state.clone(), usage_middleware))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
