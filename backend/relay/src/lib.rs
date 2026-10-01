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

pub mod agent_card;
pub mod auth;
pub mod compliance;
pub mod events;
pub mod forwarder;
pub mod handlers;
pub mod inbox_bridge;
pub mod jwt_mint;
pub mod limits;
pub mod policy;
pub mod state;
pub mod telemetry;

pub use state::RelayState;

/// Usage-metering middleware: records one `usage_events` row per REST
/// request (every GET/POST/PATCH/DELETE), attributed to the caller. The
/// `/mcp` endpoint meters per-tool inside its dispatcher instead, and
/// health/well-known probes are skipped as noise.
///
/// Adds no waiting to the request: it doesn't authenticate the caller (the
/// raw `Authorization` header is resolved later by the background writer)
/// and doesn't await the INSERT — `record` is a non-blocking channel send.
async fn usage_middleware(State(state): State<RelayState>, req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());
    let skip = route == "/mcp"
        || route.starts_with("/healthz")
        || route.starts_with("/readyz")
        || route.starts_with("/.well-known");
    if skip {
        return next.run(req).await;
    }

    let method = req.method().as_str().to_owned();
    let actor = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map_or(events::UsageActor::Anonymous, |h| {
            events::UsageActor::Header(h.to_owned())
        });

    let resp = next.run(req).await;

    state.usage.record(events::UsageEvent {
        actor,
        account_id: None,
        surface: "rest",
        action: format!("{method} {route}"),
        method,
        route,
        status_code: i32::from(resp.status().as_u16()),
    });
    resp
}

/// On a 401, name the protected-resource metadata in `WWW-Authenticate`, as
/// the MCP authorization spec requires: it's how an MCP client finds where
/// to sign in.
async fn www_authenticate(State(state): State<RelayState>, req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    if resp.status() == axum::http::StatusCode::UNAUTHORIZED
        && !resp
            .headers()
            .contains_key(axum::http::header::WWW_AUTHENTICATE)
    {
        let challenge = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
            state.config.relay_base_url.trim_end_matches('/')
        );
        if let Ok(value) = axum::http::HeaderValue::from_str(&challenge) {
            resp.headers_mut()
                .insert(axum::http::header::WWW_AUTHENTICATE, value);
        }
    }
    resp
}

pub fn router(state: RelayState) -> Router {
    // Browser-based MCP clients need to read `WWW-Authenticate` on a 401.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers([axum::http::header::WWW_AUTHENTICATE]);

    let router = Router::new()
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
        // ─── Audit + usage (migration 0025) ────────────
        .route("/v1/audit", get(handlers::events_read::audit_list))
        .route("/v1/usage/events", get(handlers::events_read::usage_list))
        // ─── MCP server ────────────────────────────────
        .route(
            "/.well-known/oauth-protected-resource",
            get(handlers::mcp::protected_resource_metadata),
        )
        // RFC 9728's path form for the `/mcp` resource, which stricter MCP
        // clients look up first.
        .route(
            "/.well-known/oauth-protected-resource/mcp",
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
        .layer(from_fn_with_state(state.clone(), www_authenticate))
        .with_state(state)
        .layer(cors);
    chakramcp_shared::telemetry::instrument(router, "relay")
}

#[cfg(test)]
mod discovery_tests {
    //! How an MCP client finds where to sign in: a 401 names the
    //! protected-resource metadata, which is served at both paths.
    use axum::body::Body;
    use axum::http::{header, Method, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:8080".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn an_unauthenticated_mcp_call_points_at_the_metadata(pool: PgPool) {
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, "https://client.example")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            res.headers()[header::WWW_AUTHENTICATE],
            r#"Bearer resource_metadata="http://localhost:8090/.well-known/oauth-protected-resource""#
        );
        let exposed = res.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS]
            .to_str()
            .unwrap()
            .to_lowercase();
        assert!(exposed.contains("www-authenticate"), "{exposed}");

        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/mcp",
        ] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(doc["resource"], "http://localhost:8090/mcp");
            assert_eq!(doc["authorization_servers"][0], "http://localhost:8080");
        }
    }
}
