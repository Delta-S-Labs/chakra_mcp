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
             redirect_uri, scope, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
               expires_at, used_at, revoked_at
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
               consumed_at, revoked_at
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
                 agent_description_hint, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
               agent_description_hint, expires_at, approved_at, denied_at,
               consumed_at
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
    pub agent_slug: String,
    pub agent_display_name: String,
    #[serde(default)]
    pub agent_description: Option<String>,
    #[serde(default)]
    pub agent_visibility: Option<String>,
    #[serde(default)]
    pub account_slug: Option<String>,
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
    let slug = req.agent_slug.trim().to_lowercase();
    let display_name = req.agent_display_name.trim().to_string();
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
    let visibility = req.agent_visibility.as_deref().unwrap_or("private");
    if !matches!(visibility, "private" | "network") {
        return Err(ApiError::InvalidRequest(
            "agent_visibility must be 'private' or 'network'".into(),
        ));
    }

    let mut tx = state.db.begin().await?;

    let row = sqlx::query!(
        r#"
        SELECT id, expires_at, approved_at, denied_at, consumed_at
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

    // Resolve account: explicit slug must be one the user is a member of;
    // otherwise pick their personal (account_type='individual') account.
    // The two query! macros generate distinct anonymous structs — unify
    // into a plain (Uuid, String) tuple at the boundary.
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
            ApiError::Conflict("user has no personal account; pass account_slug explicitly".into())
        })?;
        (acc.id, acc.slug)
    };

    // Insert the agent. Pull-mode (no endpoint_url / agent_card_url) is
    // the right default for device-flow agents — they reach the relay
    // via the inbox bridge, not via an inbound public host.
    let agent_id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agents
            (id, account_id, created_by_user_id, slug, display_name, description, visibility, mode)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pull')
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
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        ApiError::Conflict(format!(
            "agent slug '{slug}' already exists in this account"
        ))
    })?;

    sqlx::query!(
        r#"
        UPDATE oauth_device_codes
        SET approved_user_id = $2, approved_agent_id = $3, approved_at = now()
        WHERE id = $1
        "#,
        row.id,
        user.user_id,
        inserted.id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(DeviceApproveResponse {
        status: "approved",
        agent_id: inserted.id,
        agent_slug: inserted.slug,
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
