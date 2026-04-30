//! A2A JSON-RPC + streaming endpoint stubs.
//!
//! D2 publishes Agent Cards whose `supported_interfaces[].url` points
//! at these routes. Until D4/D5 land the real auth-decision-and-forward
//! pipeline, we expose stubs that resolve to a clean structured 501
//! rather than 404 — generic A2A clients fetching our cards and
//! attempting to call get a meaningful error code, not a routing miss.
//!
//! Lifecycle:
//! - D3 (here): both routes return 501 with `data.code = chk.not_implemented_yet`.
//! - D4: same routes gain auth-bearer parsing + the 10-step policy
//!   decision tree, still returning 501 on the success branch.
//! - D5: the success branch routes to the JWT minter + forwarder +
//!   inbox bridge — push agents get proxied, pull agents parked.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::state::RelayState;

/// `POST /agents/<account_slug>/<agent_slug>/a2a/jsonrpc`
pub async fn jsonrpc_stub(State(state): State<RelayState>) -> Response {
    if !state.config.discovery_v2_enabled {
        return not_found();
    }
    not_implemented()
}

/// `POST /agents/<account_slug>/<agent_slug>/a2a/stream`
pub async fn stream_stub(State(state): State<RelayState>) -> Response {
    if !state.config.discovery_v2_enabled {
        return not_found();
    }
    not_implemented()
}

fn not_implemented() -> Response {
    // JSON-RPC 2.0 envelope with our stable error code in `data.code`.
    // Callers resolve the catalog at /.well-known/error-codes.json
    // (D12) for human-readable details. Until then, the code is the
    // contract.
    (
        StatusCode::NOT_IMPLEMENTED,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32601,
                "message": "Method not implemented yet",
                "data": {
                    "code": "chk.not_implemented_yet",
                    "ships_in": "D5",
                }
            }
        })),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "error": "A2A endpoint not enabled",
            "code": "chk.discovery_v2_disabled",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn config(v2: bool) -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: v2,
            log_filter: "warn".into(),
        }
    }

    /// Both stubs respond with HTTP 501 and a structured JSON-RPC
    /// 2.0 error envelope carrying `data.code = chk.not_implemented_yet`.
    /// Generic A2A clients fetching our cards and trying to call
    /// today get this meaningful error rather than 404.
    #[sqlx::test(migrations = "../migrations")]
    async fn jsonrpc_stub_returns_501_with_structured_error(pool: PgPool) {
        let state = crate::state::RelayState::new(pool, config(true));
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents/acme-corp/alice/a2a/jsonrpc")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","method":"SendMessage","id":1}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["error"]["code"], -32601);
        assert_eq!(v["error"]["data"]["code"], "chk.not_implemented_yet");
        assert_eq!(v["error"]["data"]["ships_in"], "D5");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn stream_stub_returns_501(pool: PgPool) {
        let state = crate::state::RelayState::new(pool, config(true));
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/agents/acme/alice/a2a/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_404_when_v2_disabled(pool: PgPool) {
        let state = crate::state::RelayState::new(pool, config(false));
        let app = crate::router(state);
        for path in [
            "/agents/acme/alice/a2a/jsonrpc",
            "/agents/acme/alice/a2a/stream",
        ] {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "path: {path}");
        }
    }

    /// The card published by D2 contains a `url` field — that URL
    /// MUST resolve to a real (stub) route. This test:
    ///   1. seeds a pull-mode agent + capability
    ///   2. fetches the card via the D2c handler
    ///   3. extracts the URL from the card
    ///   4. POSTs to it
    ///   5. asserts 501 (not 404)
    /// — proving D2's published URL pattern and D3's route registration agree.
    #[sqlx::test(migrations = "../migrations")]
    async fn card_url_actually_resolves(pool: PgPool) {
        let acct_id = uuid::Uuid::now_v7();
        let agent_id = uuid::Uuid::now_v7();
        let cap_id = uuid::Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, 'acme-corp', 'Acme', 'individual')"#,
            acct_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'alice', 'Alice', 'network')"#,
            agent_id,
            acct_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb, 'network')"#,
            cap_id,
            agent_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let state = crate::state::RelayState::new(pool, config(true));
        let app = crate::router(state);

        // 1. Fetch the card.
        let card_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/alice/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(card_res.status(), StatusCode::OK);
        let body = card_res.into_body().collect().await.unwrap().to_bytes();
        let card: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let url_in_card = card["supported_interfaces"][0]["url"].as_str().unwrap();
        // The card publishes the absolute URL; for the in-process
        // router test we strip the host and just call the path.
        let path = url_in_card
            .trim_start_matches("http://localhost:8090")
            .trim_start_matches("https://localhost:8090");
        assert!(
            path.starts_with("/agents/acme-corp/alice/a2a/jsonrpc"),
            "unexpected card url: {url_in_card}"
        );

        // 2. POST to that path. Expect 501 (not 404).
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","method":"SendMessage","id":1}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);
    }
}

