//! OAuth 2.1 + PKCE authorization server.
//!
//! Endpoints:
//!   * GET  /.well-known/oauth-authorization-server (RFC 8414 metadata)
//!   * POST /oauth/register      (RFC 7591 dynamic client registration)
//!   * POST /oauth/issue-code    (called by frontend after user consent)
//!   * POST /oauth/token         (RFC 6749 token endpoint)
//!
//! Auth codes are stored hashed (sha256) and one-shot. Access tokens
//! are the same JWT shape as user-session JWTs — the relay's existing
//! Bearer auth path validates them transparently.

use axum::extract::State;
use axum::Json;
use chrono::{Duration, Utc};
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

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn random_token(byte_len: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; byte_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        &bytes,
    )
}

// ─── GET /.well-known/oauth-authorization-server ─────────
pub async fn metadata(State(state): State<AppState>) -> Json<Value> {
    let cfg = &state.config;
    Json(json!({
        "issuer": cfg.app_base_url,
        "authorization_endpoint": format!("{}/oauth/authorize", cfg.frontend_base_url),
        "token_endpoint": format!("{}/oauth/token", cfg.app_base_url),
        "registration_endpoint": format!("{}/oauth/register", cfg.app_base_url),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
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
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub code_verifier: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub scope: String,
}

pub async fn token(
    State(state): State<AppState>,
    axum::Form(req): axum::Form<TokenRequest>,
) -> ApiResult<Json<TokenResponse>> {
    if req.grant_type != "authorization_code" {
        return Err(ApiError::InvalidRequest(
            "only grant_type=authorization_code is supported".into(),
        ));
    }
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
               expires_at, used_at
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

    sqlx::query!(
        r#"UPDATE oauth_authorizations SET used_at = now() WHERE id = $1"#,
        row.id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Mint an access token. Same JWT shape as the rest of the system —
    // the relay validates it via the existing Bearer path.
    let user = sqlx::query!(
        r#"SELECT email, is_admin FROM users WHERE id = $1"#,
        row.user_id,
    )
    .fetch_one(&state.db)
    .await?;

    let claims = jwt::UserClaims::new(row.user_id, user.email, user.is_admin, ACCESS_TOKEN_TTL_HOURS);
    let access_token = jwt::encode_jwt(&claims, &state.config.jwt_secret)?;

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: ACCESS_TOKEN_TTL_HOURS * 3600,
        scope: row.scope,
    }))
}
