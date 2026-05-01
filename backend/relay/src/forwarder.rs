//! Push-mode forwarder (D5b).
//!
//! When the policy gate (D4) authorizes an A2A call to a push-mode
//! target, this module:
//!
//! 1. Resolves the target's actual canonical A2A endpoint from the
//!    cached upstream Agent Card (D2d's `CachedCardEnvelope.upstream`).
//! 2. Mints a short-lived relay-issued JWT (D5a) carrying trust
//!    context.
//! 3. POSTs the original A2A request body verbatim to the upstream
//!    endpoint with `Authorization: Bearer <jwt>`.
//! 4. Persists a `relay_invocations` row with the outcome.
//! 5. Returns the upstream response (status + body) to the caller.
//!
//! The forwarder is a *transparent proxy* at the A2A semantic layer:
//! it doesn't interpret SendMessage / GetTask / etc. The whole point
//! of the policy gate is that once a call is authorized, we don't
//! re-derive A2A semantics at the relay — we trust the wire.
//!
//! Errors on this path map to `chk.target.unreachable` for the
//! caller (the policy gate already caught everything else).

use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use bytes::Bytes;
use chrono::Utc;
use reqwest::header;
use sqlx::PgPool;
use uuid::Uuid;

use crate::agent_card::{
    fetcher::CachedCardEnvelope,
    keys::KeyStore,
    types::AgentCard,
};
use crate::jwt_mint::{mint_for_proxied_call, DEFAULT_TTL_SECONDS};
use crate::policy::Authorized;

/// Hard cap on how long we wait for an upstream agent to respond.
/// Long enough for non-trivial agent work (LLM calls, tool use)
/// but bounded so a stuck upstream doesn't pin a request thread.
pub const FORWARD_TIMEOUT_SECONDS: u64 = 60;

/// Outcome of a successful HTTP round-trip to the upstream agent.
/// HTTP status + body are passed through verbatim to the caller.
#[derive(Debug)]
pub struct ForwardOutcome {
    pub http_status: u16,
    pub body: Bytes,
    pub content_type: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ForwardError {
    #[error("target has no cached upstream card; cannot forward")]
    NoCachedCard,
    #[error("upstream card has no callable interface")]
    NoUpstreamInterface,
    #[error("active signing key unavailable: {0}")]
    KeyStore(String),
    #[error("failed to mint JWT: {0}")]
    Mint(String),
    #[error("network/transport error: {0}")]
    Transport(String),
    #[error("audit-row write failed: {0}")]
    Audit(#[from] sqlx::Error),
}

/// Forward an authorized A2A call to a push-mode target.
///
/// `request_body` is the raw bytes the caller POSTed to our relay's
/// `/a2a/jsonrpc` endpoint — typically a JSON-RPC envelope; we don't
/// parse or rewrite it. `request_headers` is consulted for content
/// type only; we mint the bearer ourselves.
pub async fn forward_push(
    db: &PgPool,
    keystore: &KeyStore,
    http: &reqwest::Client,
    authz: &Authorized,
    capability_name: &str,
    request_body: Bytes,
    request_headers: &HeaderMap,
) -> Result<ForwardOutcome, ForwardError> {
    debug_assert!(authz.target_is_push, "forward_push called for non-push target");

    // (1) Resolve upstream endpoint from the cached card.
    let upstream_url = load_upstream_endpoint(db, authz.target_agent_id).await?;

    // (2) Mint the relay JWT.
    let signing_key = keystore
        .ensure_active_key()
        .await
        .map_err(|e| ForwardError::KeyStore(e.to_string()))?;
    let jwt = mint_for_proxied_call(authz, &signing_key, Utc::now(), DEFAULT_TTL_SECONDS)
        .map_err(|e| ForwardError::Mint(e.to_string()))?;

    // (3) Build the upstream request, preserving content type.
    let content_type = request_headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    let invocation_id = Uuid::now_v7();
    let started = Instant::now();
    let request = http
        .post(&upstream_url)
        .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
        .header(header::CONTENT_TYPE, content_type.clone())
        .timeout(Duration::from_secs(FORWARD_TIMEOUT_SECONDS))
        .body(request_body.clone());

    // (4) Send + outcome classification.
    let outcome = match request.send().await {
        Ok(resp) => {
            let elapsed_ms = started.elapsed().as_millis() as i64;
            let status = resp.status().as_u16();
            let upstream_ct = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let body = resp
                .bytes()
                .await
                .map_err(|e| ForwardError::Transport(e.to_string()))?;
            persist_invocation(
                db,
                invocation_id,
                authz,
                capability_name,
                if (200..300).contains(&status) {
                    "succeeded"
                } else {
                    "failed"
                },
                Some(status as i32),
                elapsed_ms,
                None,
                preview_request(&request_body),
                preview_response(&body),
            )
            .await?;
            Ok(ForwardOutcome {
                http_status: status,
                body,
                content_type: upstream_ct,
            })
        }
        Err(e) => {
            let elapsed_ms = started.elapsed().as_millis() as i64;
            let is_timeout = e.is_timeout();
            persist_invocation(
                db,
                invocation_id,
                authz,
                capability_name,
                if is_timeout { "timeout" } else { "failed" },
                None,
                elapsed_ms,
                Some(format!("upstream transport error: {e}")),
                preview_request(&request_body),
                None,
            )
            .await?;
            Err(ForwardError::Transport(e.to_string()))
        }
    };

    outcome
}

/// Load the target agent's cached upstream Agent Card and extract
/// the first callable interface URL. Returns NoCachedCard if D2d
/// hasn't run for this agent yet (the caller in handlers/a2a.rs is
/// expected to lazy-fetch first via D2c's cached_or_fetch_push_card
/// before reaching here).
async fn load_upstream_endpoint(db: &PgPool, agent_id: Uuid) -> Result<String, ForwardError> {
    let row = sqlx::query!(
        "SELECT agent_card_cached FROM agents WHERE id = $1",
        agent_id,
    )
    .fetch_one(db)
    .await
    .map_err(ForwardError::Audit)?;

    let cached = row.agent_card_cached.ok_or(ForwardError::NoCachedCard)?;
    let envelope: CachedCardEnvelope =
        serde_json::from_value(cached).map_err(|_| ForwardError::NoCachedCard)?;

    // upstream is the *unmodified* card from the agent's host. The
    // first interface is the preferred one (per A2A v0.3 spec).
    let upstream: &AgentCard = &envelope.upstream;
    upstream
        .supported_interfaces
        .first()
        .map(|iface| iface.url.clone())
        .ok_or(ForwardError::NoUpstreamInterface)
}

#[allow(clippy::too_many_arguments)]
async fn persist_invocation(
    db: &PgPool,
    id: Uuid,
    authz: &Authorized,
    capability_name: &str,
    status: &str,
    http_status: Option<i32>,
    elapsed_ms: i64,
    error_message: Option<String>,
    input_preview: Option<serde_json::Value>,
    output_preview: Option<serde_json::Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, http_status,
             elapsed_ms, error_message, input_preview, output_preview)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
        id,
        authz.grant_id,
        authz.target_agent_id,
        authz.caller_agent_id,
        authz.capability_id,
        capability_name,
        authz.caller_user_id,
        status,
        http_status,
        elapsed_ms as i32,
        error_message,
        input_preview,
        output_preview,
    )
    .execute(db)
    .await?;
    Ok(())
}

const PREVIEW_BYTES: usize = 16 * 1024;

/// Truncate the request body to <= 16 KB and parse as JSON. If
/// parsing fails or the body is too large, store a marker.
fn preview_request(bytes: &Bytes) -> Option<serde_json::Value> {
    if bytes.len() > PREVIEW_BYTES {
        return Some(serde_json::json!({
            "_chk_truncated": true,
            "_chk_byte_count": bytes.len(),
        }));
    }
    serde_json::from_slice::<serde_json::Value>(bytes).ok()
}

fn preview_response(bytes: &Bytes) -> Option<serde_json::Value> {
    preview_request(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_card::types::{
        AgentCapabilities, AgentCard, AgentInterface, A2A_PROTOCOL_VERSION,
        PROTOCOL_BINDING_JSONRPC,
    };
    use axum::extract::State as AxumState;
    use axum::http::{HeaderMap as AxumHeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use sqlx::PgPool;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Test upstream that captures the inbound request — caller can
    /// assert what we sent (Authorization, body, content-type).
    struct CapturedRequest {
        authorization: Option<String>,
        body: Vec<u8>,
        content_type: Option<String>,
    }

    struct Upstream {
        url: String,
        last: Arc<std::sync::Mutex<Option<CapturedRequest>>>,
        hits: Arc<AtomicUsize>,
        responder: Box<
            dyn Fn(usize) -> (StatusCode, Vec<(&'static str, &'static str)>, Vec<u8>)
                + Send
                + Sync,
        >,
    }

    async fn start_upstream<R>(responder: R) -> Upstream
    where
        R: Fn(usize) -> (StatusCode, Vec<(&'static str, &'static str)>, Vec<u8>)
            + Send
            + Sync
            + 'static,
    {
        let last: Arc<std::sync::Mutex<Option<CapturedRequest>>> =
            Arc::new(std::sync::Mutex::new(None));
        let hits = Arc::new(AtomicUsize::new(0));

        #[derive(Clone)]
        struct AppState {
            last: Arc<std::sync::Mutex<Option<CapturedRequest>>>,
            hits: Arc<AtomicUsize>,
            responder: Arc<
                dyn Fn(usize) -> (StatusCode, Vec<(&'static str, &'static str)>, Vec<u8>)
                    + Send
                    + Sync,
            >,
        }

        async fn handler(
            AxumState(s): AxumState<AppState>,
            headers: AxumHeaderMap,
            body: axum::body::Bytes,
        ) -> axum::response::Response {
            let n = s.hits.fetch_add(1, Ordering::SeqCst) + 1;
            *s.last.lock().unwrap() = Some(CapturedRequest {
                authorization: headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string()),
                body: body.to_vec(),
                content_type: headers
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string()),
            });
            let (status, hdrs, body) = (s.responder)(n);
            let mut h = AxumHeaderMap::new();
            for (k, v) in hdrs {
                h.insert(
                    axum::http::HeaderName::from_static(k),
                    axum::http::HeaderValue::from_static(v),
                );
            }
            (status, h, body).into_response()
        }

        let app_state = AppState {
            last: last.clone(),
            hits: hits.clone(),
            responder: Arc::new(responder),
        };
        let app = axum::Router::new()
            .route("/a2a/jsonrpc", post(handler))
            .with_state(app_state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        Upstream {
            url: format!("http://{}/a2a/jsonrpc", addr),
            last,
            hits,
            responder: Box::new(|_| (StatusCode::OK, vec![], vec![])),
        }
    }

    /// Insert minimal test schema rows: account, push-mode agent
    /// with a cached envelope pointing at `upstream_url`, capability,
    /// caller user/account/agent, friendship, grant. Returns the
    /// Authorized struct the policy gate would have produced.
    async fn seed_authorized_for_upstream(pool: &PgPool, upstream_url: &str) -> Authorized {
        let caller_user = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Caller', 'x')"#,
            caller_user,
            format!("u-{caller_user}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();

        let caller_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Caller', 'individual', $3)"#,
            caller_account,
            format!("caller-{caller_account}"),
            caller_user,
        )
        .execute(pool)
        .await
        .unwrap();
        let caller_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'caller', 'Caller', 'network')"#,
            caller_agent,
            caller_account,
        )
        .execute(pool)
        .await
        .unwrap();

        let target_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Target', 'individual')"#,
            target_account,
            format!("target-{target_account}"),
        )
        .execute(pool)
        .await
        .unwrap();

        // Build a CachedCardEnvelope whose upstream points at the
        // test upstream's URL, then store it.
        let mut sec = BTreeMap::new();
        sec.insert(
            "x".to_string(),
            crate::agent_card::types::SecurityScheme::Other(serde_json::Map::new()),
        );
        let upstream_card = AgentCard {
            name: "Target".into(),
            description: "Test target".into(),
            supported_interfaces: vec![AgentInterface {
                url: upstream_url.to_string(),
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

        let target_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility, mode,
                                   agent_card_url, agent_card_cached)
               VALUES ($1, $2, 'target', 'Target', 'network', 'push', $3, $4)"#,
            target_agent,
            target_account,
            "https://upstream.example.com/.well-known/agent-card.json",
            envelope_json,
        )
        .execute(pool)
        .await
        .unwrap();

        let cap = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities (id, agent_id, name, description,
                                              input_schema, output_schema, visibility)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb, 'network')"#,
            cap,
            target_agent,
        )
        .execute(pool)
        .await
        .unwrap();

        let friendship = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO friendships
                  (id, proposer_agent_id, target_agent_id, status, decided_at)
               VALUES ($1, $2, $3, 'accepted', now())"#,
            friendship,
            target_agent,
            caller_agent,
        )
        .execute(pool)
        .await
        .unwrap();

        let grant = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO grants
                  (id, granter_agent_id, grantee_agent_id, capability_id, status)
               VALUES ($1, $2, $3, $4, 'active')"#,
            grant,
            target_agent,
            caller_agent,
            cap,
        )
        .execute(pool)
        .await
        .unwrap();

        Authorized {
            caller_user_id: caller_user,
            caller_account_id: caller_account,
            caller_agent_id: caller_agent,
            target_account_id: target_account,
            target_agent_id: target_agent,
            capability_id: cap,
            grant_id: grant,
            target_is_push: true,
        }
    }

    fn req_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        h
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn forwards_request_with_minted_jwt(pool: PgPool) {
        let upstream = start_upstream(|_| {
            (
                StatusCode::OK,
                vec![("content-type", "application/json")],
                br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#.to_vec(),
            )
        })
        .await;

        let authz = seed_authorized_for_upstream(&pool, &upstream.url).await;
        let keystore = KeyStore::new(pool.clone());
        // Force a key to exist so the verify check below has something
        // to verify against.
        let signing_key = keystore.ensure_active_key().await.unwrap();
        let http = reqwest::Client::new();
        let body = Bytes::from_static(br#"{"jsonrpc":"2.0","method":"SendMessage","id":1}"#);

        let outcome = forward_push(
            &pool,
            &keystore,
            &http,
            &authz,
            "do",
            body.clone(),
            &req_headers(),
        )
        .await
        .unwrap();

        assert_eq!(outcome.http_status, 200);
        assert_eq!(
            outcome.content_type.as_deref(),
            Some("application/json")
        );
        assert!(String::from_utf8_lossy(&outcome.body).contains("\"ok\":true"));

        // Upstream got our request with a bearer JWT we can verify.
        let captured = upstream.last.lock().unwrap().take().unwrap();
        let bearer = captured
            .authorization
            .as_ref()
            .and_then(|h| h.strip_prefix("Bearer "))
            .expect("missing Authorization");

        let pub_keys = keystore.jwks_keys().await.unwrap();
        let claims = crate::jwt_mint::decode_relay_jwt(bearer, &pub_keys, Utc::now()).unwrap();
        assert_eq!(claims.sub, authz.caller_agent_id.to_string());
        assert_eq!(claims.aud, authz.target_agent_id.to_string());
        assert_eq!(claims.capability_id, authz.capability_id.to_string());
        assert_eq!(claims.grant_id, authz.grant_id.to_string());

        // Body forwarded verbatim.
        assert_eq!(captured.body, body.as_ref());
        assert_eq!(captured.content_type.as_deref(), Some("application/json"));

        // Audit row written with status='succeeded'.
        let row = sqlx::query!(
            r#"SELECT status, http_status, capability_name FROM relay_invocations LIMIT 1"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.status, "succeeded");
        assert_eq!(row.http_status, Some(200));
        assert_eq!(row.capability_name, "do");

        // Silence unused-warning on signing_key.
        let _ = signing_key;
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn upstream_5xx_returns_failed_audit_status(pool: PgPool) {
        let upstream = start_upstream(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                vec![("content-type", "application/json")],
                br#"{"error":"upstream blew up"}"#.to_vec(),
            )
        })
        .await;
        let authz = seed_authorized_for_upstream(&pool, &upstream.url).await;
        let keystore = KeyStore::new(pool.clone());
        let _ = keystore.ensure_active_key().await.unwrap();
        let http = reqwest::Client::new();

        let outcome = forward_push(
            &pool,
            &keystore,
            &http,
            &authz,
            "do",
            Bytes::from_static(b"{}"),
            &req_headers(),
        )
        .await
        .unwrap();
        assert_eq!(outcome.http_status, 500);

        let row =
            sqlx::query!("SELECT status, http_status FROM relay_invocations LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.status, "failed");
        assert_eq!(row.http_status, Some(500));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn unreachable_upstream_returns_transport_error(pool: PgPool) {
        let unreachable = "http://127.0.0.1:1/a2a/jsonrpc";
        let authz = seed_authorized_for_upstream(&pool, unreachable).await;
        let keystore = KeyStore::new(pool.clone());
        let _ = keystore.ensure_active_key().await.unwrap();
        let http = reqwest::Client::new();

        let r = forward_push(
            &pool,
            &keystore,
            &http,
            &authz,
            "do",
            Bytes::from_static(b"{}"),
            &req_headers(),
        )
        .await;
        assert!(matches!(r, Err(ForwardError::Transport(_))));

        // An audit row should still be persisted with status='failed'.
        let row = sqlx::query!(
            "SELECT status, error_message FROM relay_invocations LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.status, "failed");
        assert!(row.error_message.is_some());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn missing_cached_card_returns_no_cached_card_error(pool: PgPool) {
        // Same fixture but clear agent_card_cached after seeding.
        let authz = seed_authorized_for_upstream(&pool, "http://127.0.0.1/x").await;
        sqlx::query!(
            "UPDATE agents SET agent_card_cached = NULL WHERE id = $1",
            authz.target_agent_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let keystore = KeyStore::new(pool.clone());
        let _ = keystore.ensure_active_key().await.unwrap();
        let r = forward_push(
            &pool,
            &keystore,
            &reqwest::Client::new(),
            &authz,
            "do",
            Bytes::from_static(b"{}"),
            &req_headers(),
        )
        .await;
        assert!(matches!(r, Err(ForwardError::NoCachedCard)));
    }
}
