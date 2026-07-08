//! OAuth 2.1 + PKCE authorization server.
//!
//! Endpoints:
//!   * GET  /.well-known/oauth-authorization-server (RFC 8414 metadata)
//!   * POST /oauth/register             (RFC 7591 dynamic client registration)
//!   * POST /oauth/issue-code           (called by frontend after user consent)
//!   * POST /oauth/token                (RFC 6749 token endpoint, also RFC 8628)
//!   * POST /oauth/device_authorization (RFC 8628 device authorization grant)
//!   * GET  /oauth/device-session/{user_code}  (consent-page lookup)
//!   * POST /oauth/device-approve       (frontend → mints agent + binds device)
//!   * POST /oauth/device-deny          (frontend → marks denied)
//!
//! Auth codes (and device codes) are stored hashed (sha256) and one-shot.
//! Access tokens are the same JWT shape as user-session JWTs — the relay's
//! existing Bearer auth path validates them transparently.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};
use chakramcp_shared::jwt;

use crate::auth::AuthUser;
use crate::state::AppState;

const AUTH_CODE_TTL_MINUTES: i64 = 10;
const ACCESS_TOKEN_TTL_HOURS: i64 = 24;
const DEVICE_CODE_TTL_MINUTES: i64 = 10;
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn random_token(byte_len: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; byte_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &bytes)
}

/// Materialise a chosen agent-management scope into `credential_scopes`
/// (migration 0027), bound to a freshly-minted JWT `jti`.
///
/// A no-op for the `all` default — `resolve_grant` treats a missing row as
/// unrestricted, so we persist only genuinely-narrowing grants. Runs inside
/// the caller's token-mint transaction so the scope and the token's
/// `minted_jti` commit atomically (a half-written scope would be a security
/// hole). `selected_agent_ids` are assumed already validated by the consent
/// endpoint that stashed them.
async fn write_jwt_agent_scope(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    jti: Uuid,
    client_id: &str,
    user_id: Uuid,
    agent_scope: &str,
    selected_agent_ids: Option<&[Uuid]>,
) -> Result<(), ApiError> {
    if agent_scope == "all" {
        return Ok(());
    }
    let scope_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO credential_scopes (id, cred_kind, cred_ref, client_id, user_id, agent_scope)
        VALUES ($1, 'jwt', $2, $3, $4, $5)
        "#,
        scope_id,
        jti.to_string(),
        client_id,
        user_id,
        agent_scope,
    )
    .execute(&mut **tx)
    .await?;
    if agent_scope == "selected" {
        for aid in selected_agent_ids.unwrap_or(&[]) {
            sqlx::query!(
                r#"
                INSERT INTO credential_scope_agents (credential_scope_id, agent_id)
                VALUES ($1, $2)
                ON CONFLICT DO NOTHING
                "#,
                scope_id,
                aid,
            )
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

// ─── GET /.well-known/oauth-authorization-server ─────────
pub async fn metadata(State(state): State<AppState>) -> Json<Value> {
    let cfg = &state.config;
    Json(json!({
        "issuer": cfg.app_base_url,
        "authorization_endpoint": format!("{}/oauth/authorize", cfg.frontend_base_url),
        "token_endpoint": format!("{}/oauth/token", cfg.app_base_url),
        "registration_endpoint": format!("{}/oauth/register", cfg.app_base_url),
        "device_authorization_endpoint": format!("{}/oauth/device_authorization", cfg.app_base_url),
        "response_types_supported": ["code"],
        "grant_types_supported": [
            "authorization_code",
            DEVICE_GRANT_TYPE,
        ],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["relay.full"],
    }))
}

// ─── POST /oauth/register (RFC 7591) ─────────────────────
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// At least one redirect URI is required.
    pub redirect_uris: Vec<String>,
    pub client_name: Option<String>,
    pub client_uri: Option<String>,
    /// Most MCP clients are public (no client_secret) and use PKCE.
    /// We don't accept "client_secret_basic" / "client_secret_post" yet.
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub client_id: String,
    pub client_id_issued_at: i64,
    pub redirect_uris: Vec<String>,
    pub client_name: String,
    pub token_endpoint_auth_method: &'static str,
    pub grant_types: Vec<&'static str>,
    pub response_types: Vec<&'static str>,
    pub scope: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Json<RegisterResponse>> {
    if req.redirect_uris.is_empty() {
        return Err(ApiError::InvalidRequest(
            "redirect_uris must contain at least one URI".into(),
        ));
    }
    for uri in &req.redirect_uris {
        if !(uri.starts_with("http://") || uri.starts_with("https://")) {
            return Err(ApiError::InvalidRequest(format!(
                "redirect_uri '{uri}' must use http or https"
            )));
        }
    }
    // Reject confidential-client registration for now — MCP hosts are public.
    if let Some(method) = req.token_endpoint_auth_method.as_deref() {
        if method != "none" {
            return Err(ApiError::InvalidRequest(
                "only token_endpoint_auth_method=none (PKCE) is supported".into(),
            ));
        }
    }

    let client_name = req
        .client_name
        .unwrap_or_else(|| "Unnamed MCP client".to_string());
    let scope = req.scope.unwrap_or_else(|| "relay.full".to_string());
    if scope != "relay.full" {
        return Err(ApiError::InvalidRequest(
            "only scope=relay.full is supported".into(),
        ));
    }

    let id = Uuid::now_v7();
    let client_id = format!("mcp_{}", random_token(18));

    sqlx::query!(
        r#"
        INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris, scope, client_uri)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
        id,
        client_id,
        client_name,
        &req.redirect_uris,
        scope,
        req.client_uri,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(RegisterResponse {
        client_id: client_id.clone(),
        client_id_issued_at: Utc::now().timestamp(),
        redirect_uris: req.redirect_uris,
        client_name,
        token_endpoint_auth_method: "none",
        grant_types: vec!["authorization_code"],
        response_types: vec!["code"],
        scope,
    }))
}

// ─── GET /oauth/clients/{client_id} (consent UI lookup) ──
//
// Tiny helper the frontend calls so the consent page can show the
// client's name without needing to be told by the (potentially
// untrusted) URL params.
#[derive(Debug, Serialize)]
pub struct ClientPreview {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub client_uri: Option<String>,
    pub scope: String,
}

pub async fn get_client(
    State(state): State<AppState>,
    axum::extract::Path(client_id): axum::extract::Path<String>,
) -> ApiResult<Json<ClientPreview>> {
    let row = sqlx::query!(
        r#"
        SELECT client_id, client_name, redirect_uris, client_uri, scope
        FROM oauth_clients WHERE client_id = $1
        "#,
        client_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(ClientPreview {
        client_id: row.client_id,
        client_name: row.client_name,
        redirect_uris: row.redirect_uris,
        client_uri: row.client_uri,
        scope: row.scope,
    }))
}

// ─── POST /oauth/issue-code ──────────────────────────────
//
// Called by the frontend after the user clicks "approve" on the
// consent page. The frontend authenticates as the user with their
// backend JWT and the backend mints an auth code. The frontend then
// redirects the user's browser to the client's redirect_uri with the
// code attached.
#[derive(Debug, Deserialize)]
pub struct IssueCodeRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    #[serde(default = "default_method")]
    pub code_challenge_method: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Agent-management scope the user granted at consent (scoped-agent
    /// -grants): "all" (default — unrestricted), "own" (only agents this
    /// client creates), or "selected". Orthogonal to the OAuth `scope`
    /// above (which stays `relay.full`).
    #[serde(default)]
    pub agent_scope: Option<String>,
    /// Explicit agent allow-list when `agent_scope = "selected"`. Each id
    /// must be an agent in an account the consenting user belongs to.
    #[serde(default)]
    pub selected_agent_ids: Option<Vec<Uuid>>,
}

fn default_method() -> String {
    "S256".to_string()
}
fn default_scope() -> String {
    "relay.full".to_string()
}

#[derive(Debug, Serialize)]
pub struct IssueCodeResponse {
    pub code: String,
    pub expires_in: i64,
}

pub async fn issue_code(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<IssueCodeRequest>,
) -> ApiResult<Json<IssueCodeResponse>> {
    if req.code_challenge_method != "S256" {
        return Err(ApiError::InvalidRequest(
            "code_challenge_method must be S256".into(),
        ));
    }
    if req.code_challenge.len() < 43 || req.code_challenge.len() > 128 {
        return Err(ApiError::InvalidRequest(
            "code_challenge length must be between 43 and 128".into(),
        ));
    }
    if req.scope != "relay.full" {
        return Err(ApiError::InvalidRequest(
            "only scope=relay.full is supported".into(),
        ));
    }

    // Agent-management scope (scoped-agent-grants). Validate the mode and,
    // for 'selected', that every named agent lives in an account the
    // consenting user belongs to — a grant can only ever narrow within what
    // the user can already manage. Stashed on the authorization row and
    // materialised into credential_scopes when the token is minted.
    let agent_scope = req.agent_scope.as_deref().unwrap_or("all");
    if !matches!(agent_scope, "all" | "own" | "selected") {
        return Err(ApiError::InvalidRequest(
            "agent_scope must be all|own|selected".into(),
        ));
    }
    let mut selected_ids = req.selected_agent_ids.clone().unwrap_or_default();
    selected_ids.sort();
    selected_ids.dedup();
    if agent_scope == "selected" {
        if selected_ids.is_empty() {
            return Err(ApiError::InvalidRequest(
                "selected_agent_ids is required when agent_scope=selected".into(),
            ));
        }
        let in_scope = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "n!"
            FROM agents a
            JOIN account_memberships m ON m.account_id = a.account_id
            WHERE m.user_id = $1 AND a.id = ANY($2)
            "#,
            user.user_id,
            &selected_ids,
        )
        .fetch_one(&state.db)
        .await?;
        if in_scope as usize != selected_ids.len() {
            return Err(ApiError::InvalidRequest(
                "selected_agent_ids must all be agents in your accounts".into(),
            ));
        }
    }
    let stored_ids: Option<&[Uuid]> = if agent_scope == "selected" {
        Some(&selected_ids)
    } else {
        None
    };

    let client = sqlx::query!(
        r#"SELECT redirect_uris FROM oauth_clients WHERE client_id = $1"#,
        req.client_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !client.redirect_uris.iter().any(|u| u == &req.redirect_uri) {
        return Err(ApiError::InvalidRequest(
            "redirect_uri does not match any registered URI for this client".into(),
        ));
    }

    let code = random_token(32);
    let code_hash = sha256_hex(&code);
    let expires_at = Utc::now() + Duration::minutes(AUTH_CODE_TTL_MINUTES);

    sqlx::query!(
        r#"
        INSERT INTO oauth_authorizations
            (id, client_id, user_id, code_hash, code_challenge, code_challenge_method,
             redirect_uri, scope, expires_at, agent_scope, selected_agent_ids)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        Uuid::now_v7(),
        req.client_id,
        user.user_id,
        code_hash,
        req.code_challenge,
        req.code_challenge_method,
        req.redirect_uri,
        req.scope,
        expires_at,
        agent_scope,
        stored_ids,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(IssueCodeResponse {
        code,
        expires_in: AUTH_CODE_TTL_MINUTES * 60,
    }))
}

// ─── POST /oauth/token ───────────────────────────────────
//
// Public endpoint, accepts application/x-www-form-urlencoded per RFC 6749.
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    // authorization_code fields
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub code_verifier: Option<String>,
    // device_code field
    pub device_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub scope: String,
}

/// Build an RFC 8628 §3.5 error response: HTTP 400 + `{ "error": "<code>" }`.
///
/// We bypass `ApiError` because that envelope is `{ error: { code, message,
/// retryable } }` and RFC 8628 clients (notably the various LLM-host MCP
/// clients) expect the flat shape.
fn oauth_error(code: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": code })),
    )
        .into_response()
}

pub async fn token(
    State(state): State<AppState>,
    axum::Form(req): axum::Form<TokenRequest>,
) -> axum::response::Response {
    match req.grant_type.as_str() {
        "authorization_code" => match token_authorization_code(&state, &req).await {
            Ok(resp) => Json(resp).into_response(),
            Err(e) => e.into_response(),
        },
        DEVICE_GRANT_TYPE => token_device_code(&state, &req).await,
        _ => ApiError::InvalidRequest(format!("unsupported grant_type '{}'", req.grant_type))
            .into_response(),
    }
}

async fn token_authorization_code(
    state: &AppState,
    req: &TokenRequest,
) -> ApiResult<TokenResponse> {
    let code = req
        .code
        .as_deref()
        .ok_or_else(|| ApiError::InvalidRequest("code is required".into()))?;
    let client_id = req
        .client_id
        .as_deref()
        .ok_or_else(|| ApiError::InvalidRequest("client_id is required".into()))?;
    let redirect_uri = req
        .redirect_uri
        .as_deref()
        .ok_or_else(|| ApiError::InvalidRequest("redirect_uri is required".into()))?;
    let code_verifier = req
        .code_verifier
        .as_deref()
        .ok_or_else(|| ApiError::InvalidRequest("code_verifier is required".into()))?;

    let code_hash = sha256_hex(code);

    // Look up + atomically mark used so a second redemption fails.
    let mut tx = state.db.begin().await?;
    let row = sqlx::query!(
        r#"
        SELECT id, client_id, user_id, code_challenge, redirect_uri, scope,
               expires_at, used_at, revoked_at, agent_scope, selected_agent_ids
        FROM oauth_authorizations
        WHERE code_hash = $1
        FOR UPDATE
        "#,
        code_hash,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::InvalidRequest("invalid code".into()))?;

    if row.used_at.is_some() {
        return Err(ApiError::InvalidRequest("code already used".into()));
    }
    if row.revoked_at.is_some() {
        // /v1/pairings revoke set this — refuse to mint.
        return Err(ApiError::InvalidRequest("code revoked".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(ApiError::InvalidRequest("code expired".into()));
    }
    if row.client_id != client_id {
        return Err(ApiError::InvalidRequest("client_id mismatch".into()));
    }
    if row.redirect_uri != redirect_uri {
        return Err(ApiError::InvalidRequest("redirect_uri mismatch".into()));
    }

    // PKCE: SHA-256(code_verifier) base64url-no-pad must equal code_challenge.
    let mut h = Sha256::new();
    h.update(code_verifier.as_bytes());
    let computed = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        h.finalize(),
    );
    if computed != row.code_challenge {
        return Err(ApiError::InvalidRequest("PKCE verifier mismatch".into()));
    }

    // Mint an access token. Same JWT shape as the rest of the system —
    // the relay validates it via the existing Bearer path. We build
    // claims *before* the consume UPDATE so its jti can be persisted
    // alongside `used_at` for the /v1/pairings revoke flow.
    let user = sqlx::query!(
        r#"SELECT email, is_admin FROM users WHERE id = $1"#,
        row.user_id,
    )
    .fetch_one(&mut *tx)
    .await?;

    let claims = jwt::UserClaims::new(
        row.user_id,
        user.email,
        user.is_admin,
        ACCESS_TOKEN_TTL_HOURS,
    );

    sqlx::query!(
        r#"
        UPDATE oauth_authorizations
        SET used_at = now(), minted_jti = $2
        WHERE id = $1
        "#,
        row.id,
        claims.jti,
    )
    .execute(&mut *tx)
    .await?;

    // Bind the chosen agent-management scope to the token we just minted
    // (scoped-agent-grants), in the same tx so scope + token are atomic.
    write_jwt_agent_scope(
        &mut tx,
        claims.jti,
        &row.client_id,
        row.user_id,
        &row.agent_scope,
        row.selected_agent_ids.as_deref(),
    )
    .await?;
    tx.commit().await?;

    let access_token = jwt::encode_jwt(&claims, &state.config.jwt_secret)?;

    Ok(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: ACCESS_TOKEN_TTL_HOURS * 3600,
        scope: row.scope,
    })
}

// ─── POST /oauth/token — device_code branch ───────────────
//
// RFC 8628 §3.4–§3.5 token endpoint behaviour for device_code grants.
// All error paths return HTTP 400 + flat `{ "error": "<code>" }`.
async fn token_device_code(state: &AppState, req: &TokenRequest) -> axum::response::Response {
    let device_code = match req.device_code.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => return oauth_error("invalid_request"),
    };

    let device_code_hash = sha256_hex(device_code);

    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => return ApiError::Database(e).into_response(),
    };

    let row = match sqlx::query!(
        r#"
        SELECT id, scope, approved_user_id, approved_agent_id, approved_at,
               denied_at, expires_at, poll_interval_seconds, last_polled_at,
               consumed_at, revoked_at, client_id, agent_scope, selected_agent_ids
        FROM oauth_device_codes
        WHERE device_code_hash = $1
        FOR UPDATE
        "#,
        device_code_hash,
    )
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return oauth_error("invalid_grant"),
        Err(e) => return ApiError::Database(e).into_response(),
    };

    let now = Utc::now();

    if row.denied_at.is_some() {
        return oauth_error("access_denied");
    }
    if row.revoked_at.is_some() {
        // Operator (or the user from /v1/pairings) revoked the session
        // after approval but before — or after — a token was minted.
        // Either way, refuse to mint another.
        return oauth_error("access_denied");
    }
    if row.expires_at <= now {
        return oauth_error("expired_token");
    }
    if row.consumed_at.is_some() {
        // One-shot: token already minted against this device_code.
        return oauth_error("invalid_grant");
    }

    // Still pending consent.
    if row.approved_at.is_none() {
        let interval = row.poll_interval_seconds.max(1);
        // RFC 8628 §3.5: if the client polls faster than the interval,
        // tell it to slow down and bump the interval by 5 seconds.
        let polled_too_fast = row
            .last_polled_at
            .map(|last| now - last < Duration::seconds(interval as i64))
            .unwrap_or(false);

        let (code, new_interval) = if polled_too_fast {
            ("slow_down", interval + 5)
        } else {
            ("authorization_pending", interval)
        };

        if let Err(e) = sqlx::query!(
            r#"
            UPDATE oauth_device_codes
            SET last_polled_at = now(), poll_interval_seconds = $2
            WHERE id = $1
            "#,
            row.id,
            new_interval,
        )
        .execute(&mut *tx)
        .await
        {
            return ApiError::Database(e).into_response();
        }
        if let Err(e) = tx.commit().await {
            return ApiError::Database(e).into_response();
        }
        return oauth_error(code);
    }

    // Approved — mint a token, consume the row.
    let approved_user_id = match row.approved_user_id {
        Some(u) => u,
        None => return oauth_error("invalid_grant"),
    };
    let approved_agent_id = row.approved_agent_id;

    let user = match sqlx::query!(
        r#"SELECT email, is_admin FROM users WHERE id = $1"#,
        approved_user_id,
    )
    .fetch_one(&mut *tx)
    .await
    {
        Ok(u) => u,
        Err(e) => return ApiError::Database(e).into_response(),
    };

    // Optional join — pull the freshly-minted agent + account so the
    // agent can self-identify on first run without a follow-up call.
    let agent_info = if let Some(agent_id) = approved_agent_id {
        match sqlx::query!(
            r#"
            SELECT a.slug AS agent_slug, acc.slug AS account_slug
            FROM agents a
            JOIN accounts acc ON acc.id = a.account_id
            WHERE a.id = $1
            "#,
            agent_id,
        )
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(opt) => opt,
            Err(e) => return ApiError::Database(e).into_response(),
        }
    } else {
        None
    };

    // Build the JWT claims *before* the consume UPDATE so we can store
    // its jti alongside `consumed_at`. The /v1/pairings revoke path
    // uses `minted_jti` to insert into `revoked_tokens`, killing the
    // token before its natural 24h expiry.
    let claims = jwt::UserClaims::new(
        approved_user_id,
        user.email,
        user.is_admin,
        ACCESS_TOKEN_TTL_HOURS,
    );

    if let Err(e) = sqlx::query!(
        r#"
        UPDATE oauth_device_codes
        SET consumed_at = now(), minted_jti = $2
        WHERE id = $1
        "#,
        row.id,
        claims.jti,
    )
    .execute(&mut *tx)
    .await
    {
        return ApiError::Database(e).into_response();
    }

    // Materialise the chosen agent-management scope (scoped-agent-grants),
    // bound to the token's jti, in the same tx. 'all' writes nothing.
    // Device nuance: 'own' on an UNregistered session (no client_id) can't
    // be client-matched, so it falls back to a 'selected' scope over the
    // agent this pairing produced — device "own" always means "the agent(s)
    // this pairing created", never nothing.
    if row.agent_scope.as_str() != "all" {
        let (eff_scope, eff_ids): (&str, Vec<Uuid>) =
            if row.agent_scope.as_str() == "own" && row.client_id.is_none() {
                ("selected", row.approved_agent_id.into_iter().collect())
            } else if row.agent_scope.as_str() == "selected" {
                (
                    "selected",
                    row.selected_agent_ids.clone().unwrap_or_default(),
                )
            } else {
                ("own", Vec::new())
            };
        let scope_id = Uuid::now_v7();
        if let Err(e) = sqlx::query!(
            r#"
            INSERT INTO credential_scopes (id, cred_kind, cred_ref, client_id, user_id, agent_scope)
            VALUES ($1, 'jwt', $2, $3, $4, $5)
            "#,
            scope_id,
            claims.jti.to_string(),
            row.client_id.as_deref(),
            approved_user_id,
            eff_scope,
        )
        .execute(&mut *tx)
        .await
        {
            return ApiError::Database(e).into_response();
        }
        for aid in &eff_ids {
            if let Err(e) = sqlx::query!(
                r#"
                INSERT INTO credential_scope_agents (credential_scope_id, agent_id)
                VALUES ($1, $2)
                ON CONFLICT DO NOTHING
                "#,
                scope_id,
                aid,
            )
            .execute(&mut *tx)
            .await
            {
                return ApiError::Database(e).into_response();
            }
        }
    }

    if let Err(e) = tx.commit().await {
        return ApiError::Database(e).into_response();
    }

    let access_token = match jwt::encode_jwt(&claims, &state.config.jwt_secret) {
        Ok(t) => t,
        Err(e) => return ApiError::Auth(e).into_response(),
    };

    // Bespoke response — TokenResponse can't carry agent_id/agent_slug/account_slug.
    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": ACCESS_TOKEN_TTL_HOURS * 3600,
        "scope": row.scope,
        "agent_id": approved_agent_id,
        "agent_slug": agent_info.as_ref().map(|a| a.agent_slug.clone()),
        "account_slug": agent_info.as_ref().map(|a| a.account_slug.clone()),
    }))
    .into_response()
}

// ─── POST /oauth/device_authorization (RFC 8628 §3.1) ────
//
// Public endpoint. Returns an opaque device_code (random 32-byte
// base64url-no-pad) and a short, human-readable user_code that the
// agent prints/embeds in the verification URL it shows to the human.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthRequest {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub persona: Option<String>,
    #[serde(default)]
    pub agent_slug_hint: Option<String>,
    #[serde(default)]
    pub agent_display_name_hint: Option<String>,
    #[serde(default)]
    pub agent_description_hint: Option<String>,
    #[serde(default)]
    pub agent_visibility_hint: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    /// Public URL that renders a scannable QR for `verification_uri_complete`.
    /// Agents print this so their human can scan it from another device —
    /// no `qrencode` install or terminal-rendered ASCII art required.
    pub verification_uri_qr: String,
    pub expires_in: i64,
    pub interval: i32,
}

/// 8-char (formatted `XXXX-XXXX`) human-typable code with no visually
/// ambiguous characters (no 0/O/1/I/L). The alphabet keeps the code
/// readable when handed over via voice, screenshot, or photo.
fn generate_user_code() -> String {
    use rand::seq::SliceRandom;
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789"; // no 0/O/1/I/L
    let mut rng = rand::thread_rng();
    let pick = |rng: &mut rand::rngs::ThreadRng| {
        *ALPHABET.choose(rng).expect("alphabet is non-empty") as char
    };
    let first: String = (0..4).map(|_| pick(&mut rng)).collect();
    let second: String = (0..4).map(|_| pick(&mut rng)).collect();
    format!("{first}-{second}")
}

pub async fn device_authorization(
    State(state): State<AppState>,
    Json(req): Json<DeviceAuthRequest>,
) -> ApiResult<Json<DeviceAuthResponse>> {
    let scope = req.scope.unwrap_or_else(|| "relay.full".to_string());
    if scope != "relay.full" {
        return Err(ApiError::InvalidRequest(
            "only scope=relay.full is supported".into(),
        ));
    }

    // Optional visibility hint. It's just a pre-fill for the consent form, but
    // validate it against the set the form + device-approve accept so a bad
    // hint fails loudly here instead of getting silently dropped at approve.
    // 'org' = visible to anyone sharing an organization with the owner
    // (migration 0021); the hint CHECK is widened to match in 0031.
    if let Some(v) = req.agent_visibility_hint.as_deref() {
        if !matches!(v, "private" | "org" | "network") {
            return Err(ApiError::InvalidRequest(
                "agent_visibility_hint must be 'private', 'org', or 'network'".into(),
            ));
        }
    }

    let device_code = random_token(32);
    let device_code_hash = sha256_hex(&device_code);

    // Pick a unique user_code. Collisions are astronomically unlikely
    // (31^8 ≈ 8.5e11), but we still loop a few times to guarantee
    // termination if the cosmic dice land hard.
    let mut user_code = generate_user_code();
    let mut attempts = 0;
    let inserted = loop {
        let id = Uuid::now_v7();
        let expires_at = Utc::now() + Duration::minutes(DEVICE_CODE_TTL_MINUTES);
        let res = sqlx::query!(
            r#"
            INSERT INTO oauth_device_codes
                (id, client_id, device_code_hash, user_code, scope,
                 persona, agent_slug_hint, agent_display_name_hint,
                 agent_description_hint, agent_visibility_hint, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (user_code) DO NOTHING
            RETURNING id
            "#,
            id,
            req.client_id,
            device_code_hash,
            user_code,
            scope,
            req.persona,
            req.agent_slug_hint,
            req.agent_display_name_hint,
            req.agent_description_hint,
            req.agent_visibility_hint,
            expires_at,
        )
        .fetch_optional(&state.db)
        .await?;
        if res.is_some() {
            break user_code.clone();
        }
        attempts += 1;
        if attempts >= 5 {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "failed to allocate unique user_code after 5 attempts"
            )));
        }
        user_code = generate_user_code();
    };

    let frontend = state.config.frontend_base_url.trim_end_matches('/');
    let verification_uri = format!("{frontend}/app/pair");
    let verification_uri_complete = format!("{frontend}/app/pair?session={inserted}");
    // Public QR-render endpoint (no auth, cacheable). The url-encoded
    // `data` query is the verification_uri_complete the human will scan.
    let verification_uri_qr = format!(
        "{frontend}/qr?data={}",
        urlencoding::encode(&verification_uri_complete),
    );

    Ok(Json(DeviceAuthResponse {
        device_code,
        user_code: inserted,
        verification_uri,
        verification_uri_complete,
        verification_uri_qr,
        expires_in: DEVICE_CODE_TTL_MINUTES * 60,
        interval: 5,
    }))
}

// ─── GET /oauth/device-session/{user_code} ───────────────
//
// Frontend consent page calls this with the user's JWT to show
// agent-slug / display-name hints. Status reflects current lifecycle.
#[derive(Debug, Serialize)]
pub struct DeviceSessionResponse {
    pub persona: Option<String>,
    pub agent_slug_hint: Option<String>,
    pub agent_display_name_hint: Option<String>,
    pub agent_description_hint: Option<String>,
    pub agent_visibility_hint: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub status: &'static str,
}

pub async fn device_session(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(user_code): Path<String>,
) -> ApiResult<Json<DeviceSessionResponse>> {
    let row = sqlx::query!(
        r#"
        SELECT persona, agent_slug_hint, agent_display_name_hint,
               agent_description_hint, agent_visibility_hint, expires_at,
               approved_at, denied_at, consumed_at
        FROM oauth_device_codes
        WHERE user_code = $1
        "#,
        user_code,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let status = if row.consumed_at.is_some() {
        "consumed"
    } else if row.denied_at.is_some() {
        "denied"
    } else if row.approved_at.is_some() {
        "approved"
    } else if row.expires_at <= Utc::now() {
        "expired"
    } else {
        "pending"
    };

    Ok(Json(DeviceSessionResponse {
        persona: row.persona,
        agent_slug_hint: row.agent_slug_hint,
        agent_display_name_hint: row.agent_display_name_hint,
        agent_description_hint: row.agent_description_hint,
        agent_visibility_hint: row.agent_visibility_hint,
        expires_at: row.expires_at,
        status,
    }))
}

// ─── POST /oauth/device-approve ───────────────────────────
//
// Frontend consent page calls this after the human picks an agent
// slug + display name. We create the agent in the chosen account
// (defaulting to the user's personal account) and bind it to the
// device_code so the agent's next poll on /oauth/token sees the
// approval and mints a token.
#[derive(Debug, Deserialize)]
pub struct DeviceApproveRequest {
    pub user_code: String,
    /// Attach the device session to an EXISTING agent the user owns
    /// instead of creating a new one. When set, agent_slug /
    /// agent_display_name / visibility are ignored.
    #[serde(default)]
    pub existing_agent_id: Option<Uuid>,
    #[serde(default)]
    pub agent_slug: Option<String>,
    #[serde(default)]
    pub agent_display_name: Option<String>,
    #[serde(default)]
    pub agent_description: Option<String>,
    #[serde(default)]
    pub agent_visibility: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
    /// Agent-management scope for the token this pairing will mint
    /// (scoped-agent-grants): "all" (default), "own", or "selected".
    #[serde(default)]
    pub agent_scope: Option<String>,
    /// Agent ids when `agent_scope == "selected"`. Each must be an agent in
    /// an account the approving user belongs to.
    #[serde(default)]
    pub selected_agent_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize)]
pub struct DeviceApproveResponse {
    pub status: &'static str,
    pub agent_id: Uuid,
    pub agent_slug: String,
    pub account_slug: String,
}

/// Slug rules — kept in sync with `relay::handlers::agents::is_valid_slug`.
/// 3–32 chars of `[a-z0-9-]`, no leading/trailing or double hyphens.
fn is_valid_agent_slug(s: &str) -> bool {
    let len = s.chars().count();
    if !(3..=32).contains(&len) {
        return false;
    }
    if s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    if s.contains("--") {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub async fn device_approve(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<DeviceApproveRequest>,
) -> ApiResult<Json<DeviceApproveResponse>> {
    // slug / display_name / visibility are only needed when CREATING a new
    // agent. When attaching an existing one they're ignored.
    let creating = req.existing_agent_id.is_none();
    let slug = req
        .agent_slug
        .clone()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let display_name = req
        .agent_display_name
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let visibility = req.agent_visibility.as_deref().unwrap_or("private");
    if creating {
        if display_name.is_empty() {
            return Err(ApiError::InvalidRequest(
                "agent_display_name is required".into(),
            ));
        }
        if !is_valid_agent_slug(&slug) {
            return Err(ApiError::InvalidRequest(
                "agent_slug must be 3-32 chars of [a-z0-9-], no leading/trailing or double hyphens"
                    .into(),
            ));
        }
        if !matches!(visibility, "private" | "org" | "network") {
            return Err(ApiError::InvalidRequest(
                "agent_visibility must be 'private', 'org', or 'network'".into(),
            ));
        }
    }

    let mut tx = state.db.begin().await?;

    let row = sqlx::query!(
        r#"
        SELECT id, expires_at, approved_at, denied_at, consumed_at, client_id
        FROM oauth_device_codes
        WHERE user_code = $1
        FOR UPDATE
        "#,
        req.user_code,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.denied_at.is_some() || row.consumed_at.is_some() || row.approved_at.is_some() {
        return Err(ApiError::Conflict(
            "device session is no longer eligible for approval".into(),
        ));
    }
    if row.expires_at <= Utc::now() {
        return Err(ApiError::Conflict("device session has expired".into()));
    }

    // Resolve the account + target agent.
    //
    // When attaching an existing agent, the account is DERIVED from that
    // agent (validating the user is a member of its account) — the agent
    // may live in any account the user belongs to, not just their personal
    // one. When creating, resolve the account from an explicit slug or
    // fall back to the user's personal (account_type='individual') account.
    //
    // Pull-mode (no endpoint_url / agent_card_url) is the right default for
    // device-flow agents — they reach the relay via the inbox bridge, not
    // via an inbound public host.
    let (account_id, account_slug, final_id, final_slug) = if let Some(existing_id) =
        req.existing_agent_id
    {
        let a = sqlx::query!(
            r#"
            SELECT ag.id, ag.slug, acc.id AS account_id, acc.slug AS account_slug
            FROM agents ag
            JOIN accounts acc ON acc.id = ag.account_id
            JOIN account_memberships m ON m.account_id = acc.id
            WHERE ag.id = $1
              AND m.user_id = $2
              AND ag.tombstoned_at IS NULL
              AND acc.tombstoned_at IS NULL
            "#,
            existing_id,
            user.user_id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            ApiError::InvalidRequest("existing_agent_id is not a live agent you own".into())
        })?;
        (a.account_id, a.account_slug, a.id, a.slug)
    } else {
        let (account_id, account_slug) = if let Some(account_slug) = req.account_slug.as_deref() {
            let acc = sqlx::query!(
                r#"
                SELECT a.id, a.slug
                FROM accounts a
                JOIN account_memberships m ON m.account_id = a.id
                WHERE a.slug = $1 AND m.user_id = $2 AND a.tombstoned_at IS NULL
                "#,
                account_slug,
                user.user_id,
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError::Forbidden)?;
            (acc.id, acc.slug)
        } else {
            let acc = sqlx::query!(
                r#"
                SELECT a.id, a.slug
                FROM accounts a
                JOIN account_memberships m ON m.account_id = a.id
                WHERE m.user_id = $1
                  AND a.account_type = 'individual'
                  AND a.tombstoned_at IS NULL
                ORDER BY a.created_at ASC
                LIMIT 1
                "#,
                user.user_id,
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                ApiError::Conflict(
                    "user has no personal account; pass account_slug explicitly".into(),
                )
            })?;
            (acc.id, acc.slug)
        };

        let agent_id = Uuid::now_v7();
        let inserted = sqlx::query!(
            r#"
            INSERT INTO agents
                (id, account_id, created_by_user_id, slug, display_name, description, visibility, mode,
                 created_by_client_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pull', $8)
            ON CONFLICT (account_id, slug) WHERE tombstoned_at IS NULL DO NOTHING
            RETURNING id, slug
            "#,
            agent_id,
            account_id,
            user.user_id,
            slug,
            display_name,
            req.agent_description.clone().unwrap_or_default(),
            visibility,
            // Attribute the agent to the client that paired via this device
            // session (migration 0026) — this is the canonical "app created
            // an agent through the device flow" case for the 'own' scope.
            row.client_id.as_deref(),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!("agent slug '{slug}' already exists in this account"))
        })?;
        (account_id, account_slug, inserted.id, inserted.slug)
    };

    // Agent-management scope for the token this pairing will mint
    // (scoped-agent-grants). Validate the mode and, for 'selected', that the
    // agents are in an account the approver belongs to.
    let agent_scope = req.agent_scope.as_deref().unwrap_or("all");
    if !matches!(agent_scope, "all" | "own" | "selected") {
        return Err(ApiError::InvalidRequest(
            "agent_scope must be all|own|selected".into(),
        ));
    }
    let mut selected_ids = req.selected_agent_ids.clone().unwrap_or_default();
    selected_ids.sort();
    selected_ids.dedup();
    if agent_scope == "selected" {
        if selected_ids.is_empty() {
            return Err(ApiError::InvalidRequest(
                "selected_agent_ids is required when agent_scope=selected".into(),
            ));
        }
        let in_scope = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "n!"
            FROM agents a
            JOIN account_memberships m ON m.account_id = a.account_id
            WHERE m.user_id = $1 AND a.id = ANY($2)
            "#,
            user.user_id,
            &selected_ids,
        )
        .fetch_one(&mut *tx)
        .await?;
        if in_scope as usize != selected_ids.len() {
            return Err(ApiError::InvalidRequest(
                "selected_agent_ids must all be agents in your accounts".into(),
            ));
        }
    }
    let stored_ids: Option<&[Uuid]> = if agent_scope == "selected" {
        Some(&selected_ids)
    } else {
        None
    };

    sqlx::query!(
        r#"
        UPDATE oauth_device_codes
        SET approved_user_id = $2, approved_agent_id = $3, approved_at = now(),
            agent_scope = $4, selected_agent_ids = $5
        WHERE id = $1
        "#,
        row.id,
        user.user_id,
        final_id,
        agent_scope,
        stored_ids,
    )
    .execute(&mut *tx)
    .await?;

    // Audit the pairing (create or attach) so it shows in the write trail
    // like every other agent change. (The relay owns record_audit; here in
    // the app crate we insert directly into the shared table.)
    let (action, summary) = if creating {
        (
            "agent.create",
            format!("registered agent {final_slug} via device pairing"),
        )
    } else {
        ("agent.pair", format!("paired device to agent {final_slug}"))
    };
    sqlx::query!(
        r#"
        INSERT INTO audit_events
            (id, actor_user_id, account_id, action, resource_type, resource_id, summary, metadata)
        VALUES ($1, $2, $3, $4, 'agent', $5, $6, $7)
        "#,
        Uuid::now_v7(),
        user.user_id,
        account_id,
        action,
        final_id,
        summary,
        serde_json::json!({ "via": "device_pairing" }),
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(DeviceApproveResponse {
        status: "approved",
        agent_id: final_id,
        agent_slug: final_slug,
        account_slug,
    }))
}

// ─── POST /oauth/device-deny ──────────────────────────────
#[derive(Debug, Deserialize)]
pub struct DeviceDenyRequest {
    pub user_code: String,
}

pub async fn device_deny(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(req): Json<DeviceDenyRequest>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query!(
        r#"
        SELECT id, denied_at, approved_at, consumed_at
        FROM oauth_device_codes
        WHERE user_code = $1
        FOR UPDATE
        "#,
        req.user_code,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.approved_at.is_some() || row.consumed_at.is_some() {
        return Err(ApiError::Conflict(
            "device session is no longer eligible for denial".into(),
        ));
    }

    if row.denied_at.is_none() {
        sqlx::query!(
            r#"UPDATE oauth_device_codes SET denied_at = now() WHERE id = $1"#,
            row.id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use crate::tests_support::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use base64::Engine;
    use http_body_util::BodyExt;
    use sha2::{Digest, Sha256};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    // 'org' is a real visibility tier (migration 0021); the pairing hint CHECK
    // was widened to admit it in 0031, so a client can now pre-fill it.
    #[sqlx::test(migrations = "../migrations")]
    async fn device_authorization_accepts_org_visibility_hint(pool: PgPool) {
        let state = crate::AppState::new(pool.clone(), test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/device_authorization")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "agent_visibility_hint": "org" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "an 'org' hint must be accepted");

        let hint: Option<String> =
            sqlx::query_scalar("SELECT agent_visibility_hint FROM oauth_device_codes LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(hint.as_deref(), Some("org"), "the hint is stored verbatim");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn device_authorization_rejects_unknown_visibility_hint(pool: PgPool) {
        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/device_authorization")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "agent_visibility_hint": "public" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::BAD_REQUEST,
            "a bogus hint fails loudly here, not silently at approve time"
        );
    }

    // End-to-end: a 'selected' consent at /oauth/issue-code is carried
    // through to /oauth/token, which materialises a credential_scopes row
    // (+ the chosen agent's link) bound to the minted token. Proves the PR3
    // wiring; PR2 already covers what that row then enforces. Fixtures and
    // assertions use the runtime query API so they add nothing to the
    // .sqlx cache.
    #[sqlx::test(migrations = "../migrations")]
    async fn issue_code_selected_scope_materialises_credential_scope(pool: PgPool) {
        let (user, jwt, account) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, account, "a1", user.user_id).await;

        sqlx::query(
            "INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris) \
             VALUES ($1, 'mcp_test', 'Test', $2)",
        )
        .bind(Uuid::now_v7())
        .bind(vec!["https://app.test/cb".to_string()])
        .execute(&pool)
        .await
        .unwrap();

        let state = crate::AppState::new(pool.clone(), test_config());

        // PKCE pair (challenge = base64url-nopad(sha256(verifier))).
        let verifier = "verifier-0123456789012345678901234567890123456789";
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize());

        // 1) Consent → auth code, granting 'selected' over [agent_id].
        let issue_body = serde_json::json!({
            "client_id": "mcp_test",
            "redirect_uri": "https://app.test/cb",
            "code_challenge": challenge,
            "agent_scope": "selected",
            "selected_agent_ids": [agent_id],
        });
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/issue-code")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(issue_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "issue-code should succeed");
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let code = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_string();

        // 2) Redeem the code → token mint, which writes credential_scopes.
        let form = format!(
            "grant_type=authorization_code&code={code}\
             &client_id=mcp_test&redirect_uri=https%3A%2F%2Fapp.test%2Fcb\
             &code_verifier={verifier}"
        );
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "token mint should succeed");

        // 3) The grant was materialised: a 'selected' scope for this client,
        //    enrolling exactly the chosen agent.
        let (scope, client): (String, Option<String>) = sqlx::query_as(
            "SELECT agent_scope, client_id FROM credential_scopes WHERE user_id = $1",
        )
        .bind(user.user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(scope, "selected");
        assert_eq!(client.as_deref(), Some("mcp_test"));

        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM credential_scope_agents csa \
             JOIN credential_scopes cs ON cs.id = csa.credential_scope_id \
             WHERE cs.user_id = $1 AND csa.agent_id = $2",
        )
        .bind(user.user_id)
        .bind(agent_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1, "the chosen agent is enrolled in the selected scope");
    }

    // The device-flow nuance: 'own' on an unregistered session (no
    // client_id) is rewritten at mint to a 'selected' scope over the agent
    // the pairing produced, so it manages exactly that agent rather than
    // nothing.
    #[sqlx::test(migrations = "../migrations")]
    async fn device_own_without_client_falls_back_to_paired_agent(pool: PgPool) {
        let (user, _jwt, account) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, account, "paired", user.user_id).await;

        // Approved, unconsumed device session: NULL client_id, scope 'own'.
        let device_code = "dev-code-0123456789abcdef0123456789abcdef";
        let mut h = Sha256::new();
        h.update(device_code.as_bytes());
        let device_hash = hex::encode(h.finalize());
        sqlx::query(
            "INSERT INTO oauth_device_codes \
             (id, device_code_hash, user_code, scope, expires_at, \
              approved_user_id, approved_agent_id, approved_at, agent_scope) \
             VALUES ($1, $2, $3, 'relay.full', now() + interval '10 minutes', \
                     $4, $5, now(), 'own')",
        )
        .bind(Uuid::now_v7())
        .bind(&device_hash)
        .bind(format!("UC-{}", &Uuid::now_v7().simple().to_string()[..6]))
        .bind(user.user_id)
        .bind(agent_id)
        .execute(&pool)
        .await
        .unwrap();

        let state = crate::AppState::new(pool.clone(), test_config());
        let form = format!(
            "grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code={device_code}"
        );
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "device token mint should succeed"
        );

        // 'own' + no client_id became 'selected' over the paired agent.
        let (scope, client): (String, Option<String>) = sqlx::query_as(
            "SELECT agent_scope, client_id FROM credential_scopes WHERE user_id = $1",
        )
        .bind(user.user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(scope, "selected");
        assert!(client.is_none());

        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM credential_scope_agents csa \
             JOIN credential_scopes cs ON cs.id = csa.credential_scope_id \
             WHERE cs.user_id = $1 AND csa.agent_id = $2",
        )
        .bind(user.user_id)
        .bind(agent_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1, "the paired agent is enrolled");
    }
}
