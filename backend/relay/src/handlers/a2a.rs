//! A2A JSON-RPC + streaming endpoints with auth + policy gate (D4).
//!
//! D2 publishes Agent Cards whose `supported_interfaces[].url` points
//! at these routes. This handler runs the full policy decision tree
//! before responding:
//!
//! - Deny branches return JSON-RPC 2.0 error envelopes with the
//!   stable `data.code` from the discovery design's error catalog.
//!   Generic A2A clients learn the failure reason machine-readably.
//! - The success branch still returns 501 — D5 lands the actual
//!   forward (JWT-mint + proxy for push, inbox-bridge park for pull).

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::agent_card::keys::KeyStore;
use crate::forwarder::{forward_push, ForwardError, ForwardOutcome};
use crate::inbox_bridge::{park, ParkError};
use crate::policy::{evaluate, Authorized, Decision, DenyReason};
use crate::state::RelayState;

/// `POST /agents/<account_slug>/<agent_slug>/a2a/jsonrpc`
pub async fn jsonrpc_stub(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config.discovery_v2_enabled {
        return discovery_disabled();
    }
    let decision = evaluate(&state.db, &headers, &state, &account_slug, &agent_slug).await;
    match decision {
        Decision::Authorized(authz) => {
            // Snapshot capability_name once for the audit log so the
            // outcome row reads correctly years later even after a
            // rename (relay_invocations.capability_name is a snapshot).
            let cap_name = match capability_name(&state.db, authz.capability_id).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!(error = %e, "capability lookup failed post-authorization");
                    return internal_error();
                }
            };
            if authz.target_is_push {
                handle_authorized_push(&state, authz, cap_name, body, &headers).await
            } else {
                handle_authorized_pull(&state, authz, cap_name, body).await
            }
        }
        Decision::Denied(reason) => deny_response(&reason),
    }
}

/// `POST /agents/<account_slug>/<agent_slug>/a2a/stream`
pub async fn stream_stub(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !state.config.discovery_v2_enabled {
        return discovery_disabled();
    }
    // Streaming uses the same gate; the post-pass SSE-passthrough
    // implementation is its own design (mid-stream consent expiry,
    // cancellation, replica failover) and lands later in D5+.
    match evaluate(&state.db, &headers, &state, &account_slug, &agent_slug).await {
        Decision::Authorized(_) => not_implemented(),
        Decision::Denied(reason) => deny_response(&reason),
    }
}

/// Forward an authorized push call upstream and return the response.
async fn handle_authorized_push(
    state: &RelayState,
    authz: Authorized,
    capability_name: String,
    body: Bytes,
    headers: &HeaderMap,
) -> Response {
    let keystore = KeyStore::new(state.db.clone());
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            crate::forwarder::FORWARD_TIMEOUT_SECONDS,
        ))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    match forward_push(
        &state.db,
        &keystore,
        &http,
        &authz,
        &capability_name,
        body,
        headers,
    )
    .await
    {
        Ok(ForwardOutcome {
            http_status,
            body,
            content_type,
        }) => {
            let mut h = HeaderMap::new();
            h.insert(
                header::CONTENT_TYPE,
                content_type
                    .as_deref()
                    .and_then(|s| axum::http::HeaderValue::from_str(s).ok())
                    .unwrap_or_else(|| axum::http::HeaderValue::from_static("application/json")),
            );
            (
                StatusCode::from_u16(http_status).unwrap_or(StatusCode::OK),
                h,
                body,
            )
                .into_response()
        }
        Err(ForwardError::NoCachedCard) | Err(ForwardError::NoUpstreamInterface) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32005,
                    "message": "target agent's upstream card not cached or unreachable",
                    "data": { "code": "chk.target.unreachable" }
                }
            })),
        )
            .into_response(),
        Err(ForwardError::Transport(msg)) => {
            tracing::warn!(error = %msg, "upstream forward transport error");
            (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32005,
                        "message": "upstream agent unreachable",
                        "data": { "code": "chk.target.unreachable" }
                    }
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "forward_push internal error");
            internal_error()
        }
    }
}

/// Park an authorized pull-mode call and return an A2A Task with
/// `state: "submitted"` so the caller can begin polling via D5d's
/// GetTask handler. The granter SDK's `inbox.serve()` will pick the
/// row up on its next poll of `/v1/inbox` and post the result back
/// via `/v1/invocations/{id}/result` — both endpoints predate the
/// migration and survive unchanged.
async fn handle_authorized_pull(
    state: &RelayState,
    authz: Authorized,
    capability_name: String,
    body: Bytes,
) -> Response {
    match park(&state.db, &authz, &capability_name, body).await {
        Ok(parked) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": parked.jsonrpc_id,
                "result": parked.task,
            })),
        )
            .into_response(),
        Err(ParkError::BodyTooLarge(n)) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32600,
                    "message": "request body too large to park",
                    "data": { "code": "chk.request.too_large", "byte_count": n }
                }
            })),
        )
            .into_response(),
        Err(ParkError::Db(e)) => {
            tracing::error!(error = %e, "park failed");
            internal_error()
        }
    }
}

async fn capability_name(
    db: &sqlx::PgPool,
    capability_id: uuid::Uuid,
) -> Result<String, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT name FROM agent_capabilities WHERE id = $1",
        capability_id,
    )
    .fetch_one(db)
    .await?;
    Ok(row.name)
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
}

fn deny_response(reason: &DenyReason) -> Response {
    let http_status = jsonrpc_to_http(reason.jsonrpc_code());
    (
        http_status,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": reason.jsonrpc_code(),
                "message": reason.message(),
                "data": { "code": reason.data_code() }
            }
        })),
    )
        .into_response()
}

fn not_implemented() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32601,
                "message": "method not implemented yet",
                "data": { "code": "chk.not_implemented_yet", "ships_in": "D5" }
            }
        })),
    )
        .into_response()
}

fn discovery_disabled() -> Response {
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

/// Map our JSON-RPC error code family to an appropriate HTTP status.
/// Conventions:
/// - -32000 (auth missing) → 401
/// - -32001 (auth invalid) → 401
/// - -32002 (friendship) / -32003 (grant) → 403
/// - -32005 (unreachable) → 503
/// - -32006 (target tombstoned/missing) → 404
fn jsonrpc_to_http(code: i32) -> StatusCode {
    match code {
        -32000 | -32001 => StatusCode::UNAUTHORIZED,
        -32002 | -32003 => StatusCode::FORBIDDEN,
        -32005 => StatusCode::SERVICE_UNAVAILABLE,
        -32006 => StatusCode::NOT_FOUND,
        _ => StatusCode::OK,
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests cover every branch of the decision tree
    //! end-to-end via the production router. Each test seeds DB
    //! state precisely to land in a specific branch, fires a real
    //! HTTP request, and asserts the response code + data.code.

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config_v2_on() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: true,
            log_filter: "warn".into(),
        }
    }

    /// Standard fixture: two accounts, two agents (one per account),
    /// a capability on the target, an accepted friendship, an active
    /// grant. Returns ids the tests can manipulate to land in
    /// specific deny branches.
    struct Fixture {
        caller_user_id: Uuid,
        caller_account_id: Uuid,
        caller_agent_id: Uuid,
        target_account_slug: String,
        target_account_id: Uuid,
        target_agent_slug: String,
        target_agent_id: Uuid,
        capability_id: Uuid,
        api_key_plaintext: String,
    }

    async fn seed_full_fixture(pool: &PgPool) -> Fixture {
        // Caller user + their api key
        let caller_user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Caller', 'x')"#,
            caller_user_id,
            format!("caller-{caller_user_id}@test.local"),
        )
        .execute(pool)
        .await
        .unwrap();

        let api_key_plaintext = format!("ck_test_{caller_user_id}");
        let mut hasher = Sha256::new();
        hasher.update(api_key_plaintext.as_bytes());
        let key_hash = hex::encode(hasher.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'test', 'ck_test')"#,
            Uuid::now_v7(),
            caller_user_id,
            key_hash,
        )
        .execute(pool)
        .await
        .unwrap();

        // Caller account + membership
        let caller_account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Caller Org', 'individual', $3)"#,
            caller_account_id,
            format!("caller-acct-{caller_account_id}"),
            caller_user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            caller_account_id,
            caller_user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        let caller_agent_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'caller-agent', 'Caller Agent', 'network')"#,
            caller_agent_id,
            caller_account_id,
        )
        .execute(pool)
        .await
        .unwrap();

        // Target account + agent + capability
        let target_account_id = Uuid::now_v7();
        let target_account_slug = format!("target-acct-{target_account_id}");
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Target Org', 'individual')"#,
            target_account_id,
            target_account_slug,
        )
        .execute(pool)
        .await
        .unwrap();
        let target_agent_id = Uuid::now_v7();
        let target_agent_slug = "target-agent".to_string();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, $3, 'Target Agent', 'network')"#,
            target_agent_id,
            target_account_id,
            target_agent_slug,
        )
        .execute(pool)
        .await
        .unwrap();
        let capability_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb, 'network')"#,
            capability_id,
            target_agent_id,
        )
        .execute(pool)
        .await
        .unwrap();

        // Friendship (accepted) + Grant (active, target → caller, on this capability)
        sqlx::query!(
            r#"INSERT INTO friendships
                  (id, proposer_agent_id, target_agent_id, status, decided_at)
               VALUES ($1, $2, $3, 'accepted', now())"#,
            Uuid::now_v7(),
            target_agent_id,
            caller_agent_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO grants
                  (id, granter_agent_id, grantee_agent_id, capability_id, status)
               VALUES ($1, $2, $3, $4, 'active')"#,
            Uuid::now_v7(),
            target_agent_id,
            caller_agent_id,
            capability_id,
        )
        .execute(pool)
        .await
        .unwrap();

        Fixture {
            caller_user_id,
            caller_account_id,
            caller_agent_id,
            target_account_slug,
            target_account_id,
            target_agent_slug,
            target_agent_id,
            capability_id,
            api_key_plaintext,
        }
    }

    use sha2::{Digest, Sha256};

    fn req(
        path: &str,
        bearer: Option<&str>,
        caller_agent: Option<&str>,
        capability: Option<&str>,
    ) -> Request<Body> {
        let mut b = Request::builder().method("POST").uri(path);
        if let Some(t) = bearer {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if let Some(c) = caller_agent {
            b = b.header("X-ChakraMCP-Caller-Agent", c);
        }
        if let Some(c) = capability {
            b = b.header("X-ChakraMCP-Capability", c);
        }
        b.body(Body::empty()).unwrap()
    }

    async fn parse_body(res: Response) -> serde_json::Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn path_for(f: &Fixture) -> String {
        format!(
            "/agents/{}/{}/a2a/jsonrpc",
            f.target_account_slug, f.target_agent_slug
        )
    }

    // ── Auth branches ─────────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_auth_missing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app.oneshot(req(&path_for(&f), None, None, None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32000);
        assert_eq!(body["error"]["data"]["code"], "chk.auth.missing");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_auth_invalid(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(&path_for(&f), Some("ck_not_a_real_key"), None, None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["data"]["code"], "chk.auth.invalid");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_caller_agent_header_missing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(&path_for(&f), Some(&f.api_key_plaintext), None, None))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.auth.caller_agent_header_missing"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_caller_agent_not_owned(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        // Provide a UUID that ISN'T one of the caller's agents — use the target's.
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.target_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.auth.caller_agent_not_owned"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_capability_header_missing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.auth.capability_header_missing"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_target_account_not_found(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                "/agents/no-such-account/whatever/a2a/jsonrpc",
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.target.not_found"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_target_tombstoned(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!(
            "UPDATE agents SET tombstoned_at = now() WHERE id = $1",
            f.target_agent_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.target.tombstoned"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_capability_not_on_target(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&Uuid::now_v7().to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.target.capability_unknown"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_friendship_required(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!("DELETE FROM friendships")
            .execute(&pool)
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.policy.friendship_required"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_grant_required(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!("DELETE FROM grants")
            .execute(&pool)
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.policy.grant_required"
        );
    }

    /// Default fixture is a *pull*-mode target (mode column defaults
    /// to 'pull'). The handler now parks the call into the inbox bridge
    /// and returns an A2A Task with state="submitted" — D5d's GetTask
    /// handler completes the polling loop.
    #[sqlx::test(migrations = "../migrations")]
    async fn pull_pass_parks_and_returns_a2a_task(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&path_for(&f))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.api_key_plaintext))
                    .header("X-ChakraMCP-Caller-Agent", f.caller_agent_id.to_string())
                    .header("X-ChakraMCP-Capability", f.capability_id.to_string())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":7,"method":"SendMessage","params":{"text":"hi"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        // JSON-RPC envelope echoes the original id and embeds the Task.
        assert_eq!(body["id"], 7);
        let task = &body["result"];
        assert!(task["id"].is_string(), "Task.id present");
        assert_eq!(task["status"]["state"], "submitted");

        // Underlying invocation row is pending and assigned to the
        // target as granter (so its inbox.serve loop will pick it up).
        let row = sqlx::query!(
            r#"SELECT status, granter_agent_id, grantee_agent_id, capability_id, input_preview
                 FROM relay_invocations LIMIT 1"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.granter_agent_id, Some(f.target_agent_id));
        assert_eq!(row.grantee_agent_id, Some(f.caller_agent_id));
        assert_eq!(row.capability_id, Some(f.capability_id));
        // input_preview holds SendMessage params — the granter SDK's
        // existing `inbox.serve()` handler sees the same shape it
        // always saw (backward compat with v0.1.0 SDK contract).
        assert_eq!(
            row.input_preview,
            Some(serde_json::json!({"text": "hi"}))
        );
    }

    /// Promote the fixture's target agent to push mode by giving it
    /// an `agent_card_url` and a cached envelope pointing at the
    /// supplied upstream URL. The policy gate's TargetUnreachable
    /// check is bypassed by setting `agent_card_fetched_at = now()`.
    async fn promote_target_to_push(
        pool: &PgPool,
        f: &Fixture,
        upstream_url: &str,
    ) {
        use crate::agent_card::fetcher::CachedCardEnvelope;
        use crate::agent_card::types::{
            AgentCapabilities, AgentCard, AgentInterface, A2A_PROTOCOL_VERSION,
            PROTOCOL_BINDING_JSONRPC,
        };
        use std::collections::BTreeMap;
        let upstream_card = AgentCard {
            name: "Target".into(),
            description: "Test".into(),
            supported_interfaces: vec![AgentInterface {
                url: upstream_url.into(),
                protocol_binding: PROTOCOL_BINDING_JSONRPC.into(),
                tenant: None,
                protocol_version: A2A_PROTOCOL_VERSION.into(),
                extra: Default::default(),
            }],
            provider: None,
            version: "1.0.0".into(),
            documentation_url: None,
            capabilities: AgentCapabilities {
                streaming: Some(false),
                push_notifications: Some(false),
                extensions: vec![],
                extended_agent_card: None,
                extra: Default::default(),
            },
            security_schemes: BTreeMap::new(),
            security_requirements: vec![],
            default_input_modes: vec!["application/json".into()],
            default_output_modes: vec!["application/json".into()],
            skills: vec![],
            signatures: vec![],
            icon_url: None,
            extra: Default::default(),
        };
        let envelope = CachedCardEnvelope {
            normalized: upstream_card.clone(),
            upstream: upstream_card,
            etag: None,
        };
        let envelope_json = serde_json::to_value(&envelope).unwrap();
        sqlx::query!(
            r#"UPDATE agents
                  SET mode = 'push',
                      agent_card_url = $2,
                      agent_card_cached = $3,
                      agent_card_fetched_at = now()
                WHERE id = $1"#,
            f.target_agent_id,
            "https://upstream.example.com/.well-known/agent-card.json",
            envelope_json,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// End-to-end push: D2-cached card, D4 policy gate passes, D5b
    /// forwards through to a real test upstream, response flows back.
    #[sqlx::test(migrations = "../migrations")]
    async fn push_authorized_call_proxies_to_upstream(pool: PgPool) {
        // Stand up an upstream that captures the inbound request +
        // returns a JSON-RPC success response.
        use axum::extract::State as AxumState;
        use axum::http::StatusCode as AxumStatus;
        use axum::response::IntoResponse;
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct UpstreamState {
            last_auth: Arc<Mutex<Option<String>>>,
            hits: Arc<AtomicUsize>,
        }

        async fn upstream_handler(
            AxumState(s): AxumState<UpstreamState>,
            headers: HeaderMap,
            _body: axum::body::Bytes,
        ) -> axum::response::Response {
            s.hits.fetch_add(1, Ordering::SeqCst);
            *s.last_auth.lock().unwrap() = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            (
                AxumStatus::OK,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "application/json",
                )],
                br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#.to_vec(),
            )
                .into_response()
        }

        let upstream_state = UpstreamState {
            last_auth: Arc::new(Mutex::new(None)),
            hits: Arc::new(AtomicUsize::new(0)),
        };
        let upstream_app = axum::Router::new()
            .route("/a2a/jsonrpc", post(upstream_handler))
            .with_state(upstream_state.clone());
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_url = format!("http://{}/a2a/jsonrpc", upstream_addr);
        tokio::spawn(async move {
            axum::serve(upstream_listener, upstream_app).await.ok();
        });

        // Seed standard caller/target fixture, then promote target to push.
        let f = seed_full_fixture(&pool).await;
        promote_target_to_push(&pool, &f, &upstream_url).await;

        // Mint an active signing key (forwarder needs it). Normally
        // this happens lazily on first card-fetch; here we ensure
        // it explicitly so JWKS has something for the upstream to
        // verify against if it wanted to.
        let keystore = crate::agent_card::KeyStore::new(pool.clone());
        let _ = keystore.ensure_active_key().await.unwrap();

        // Caller POSTs to our relay's A2A endpoint. The full pipeline
        // runs: D4 policy gate -> D5b forwarder.
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        assert_eq!(body["result"]["ok"], true);

        // Upstream got our request with a relay-minted JWT.
        assert_eq!(upstream_state.hits.load(Ordering::SeqCst), 1);
        let auth = upstream_state.last_auth.lock().unwrap().clone().unwrap();
        let bearer = auth.strip_prefix("Bearer ").unwrap();
        let pub_keys = keystore.jwks_keys().await.unwrap();
        let claims = crate::jwt_mint::decode_relay_jwt(bearer, &pub_keys, chrono::Utc::now())
            .expect("upstream-bound JWT must verify against our JWKS");
        assert_eq!(claims.sub, f.caller_agent_id.to_string());
        assert_eq!(claims.aud, f.target_agent_id.to_string());
        assert_eq!(claims.capability_id, f.capability_id.to_string());

        // Audit row written.
        let row =
            sqlx::query!("SELECT status, http_status FROM relay_invocations LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.status, "succeeded");
        assert_eq!(row.http_status, Some(200));
    }

    /// Push-mode call where upstream is unreachable returns
    /// 502 + chk.target.unreachable to the caller (no leaking
    /// "the upstream is at xyz.example.com:1234" detail).
    #[sqlx::test(migrations = "../migrations")]
    async fn push_authorized_call_502s_when_upstream_unreachable(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        promote_target_to_push(&pool, &f, "http://127.0.0.1:1/a2a/jsonrpc").await;
        let _ = crate::agent_card::KeyStore::new(pool.clone())
            .ensure_active_key()
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["data"]["code"], "chk.target.unreachable");
    }

    /// Sanity that the deny path doesn't smuggle any caller info
    /// into the response payload — phishing-resistant by spec.
    #[sqlx::test(migrations = "../migrations")]
    async fn error_response_carries_no_user_info(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!("DELETE FROM friendships")
            .execute(&pool)
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        let body = parse_body(res).await;
        let body_str = serde_json::to_string(&body).unwrap();
        // Don't surface user ids, account ids, or agent UUIDs in the
        // response. The catalog (D12) provides any deep-links by code.
        assert!(!body_str.contains(&f.caller_user_id.to_string()));
        assert!(!body_str.contains(&f.caller_account_id.to_string()));
        assert!(!body_str.contains(&f.caller_agent_id.to_string()));
        assert!(!body_str.contains(&f.target_account_id.to_string()));
        assert!(!body_str.contains(&f.target_agent_id.to_string()));
    }
}
