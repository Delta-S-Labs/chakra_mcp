//! An agent's wake URL (waking pull agents §7).
//!
//! GET    /v1/agents/{id}/wake        the URL and how it's doing, never the secret
//! PUT    /v1/agents/{id}/wake        {url, format?}: set it; answers the secret once
//! DELETE /v1/agents/{id}/wake        clear it
//! POST   /v1/agents/{id}/wake/test   ping now and say how it went
//!
//! Anyone who may manage the agent: a member of its account, with the
//! credential's scope, as for `PATCH /v1/agents/{id}`. The URL may hold a bot
//! token, so it's only ever shown to them. Pull agents only: push agents get
//! their calls forwarded already.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;
use crate::wake_urls;

#[derive(Debug, Deserialize)]
pub struct SetWake {
    pub url: String,
    /// `json` (the default, signed) or `text` (a line, for ntfy and chat bots).
    #[serde(default)]
    pub format: Option<String>,
}

/// The agent's mode, once the caller may manage it.
async fn manageable(state: &RelayState, user: &AuthUser, id: Uuid) -> ApiResult<String> {
    let row = sqlx::query!(
        "SELECT account_id, mode FROM agents WHERE id = $1 AND tombstoned_at IS NULL",
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !user_is_member(&state.db, user.user_id, row.account_id).await? {
        return Err(ApiError::Forbidden);
    }
    crate::auth::require_scope(state, user, id).await?;
    Ok(row.mode)
}

/// `GET /v1/agents/{id}/wake`
pub async fn get(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    manageable(&state, &user, id).await?;
    Ok(Json(read(&state, id).await?.ok_or(ApiError::NotFound)?))
}

/// `PUT /v1/agents/{id}/wake`
pub async fn set(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<SetWake>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        set_wake(&state, &user, id, &req.url, req.format.as_deref()).await?,
    ))
}

/// `DELETE /v1/agents/{id}/wake`
pub async fn clear(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    clear_wake(&state, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/agents/{id}/wake/test`
pub async fn test(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    manageable(&state, &user, id).await?;
    Ok(Json(json!(wake_urls::ping_now(&state, id).await?)))
}

/// Set agent `id`'s wake URL: checked, turned on with its failures
/// cleared, and a new secret, which the answer carries this once. Shared
/// with MCP `update_agent`.
pub(crate) async fn set_wake(
    state: &RelayState,
    user: &AuthUser,
    id: Uuid,
    url: &str,
    format: Option<&str>,
) -> ApiResult<Value> {
    let mode = manageable(state, user, id).await?;
    if !state.wake_urls.enabled {
        return Err(ApiError::Conflict(
            "wake URLs are off on this server (WAKE_URLS_ENABLED=false)".into(),
        ));
    }
    if mode == "push" {
        return Err(ApiError::InvalidRequest(
            "wake URLs are for pull agents; a push agent gets its calls forwarded".into(),
        ));
    }
    let format = format.unwrap_or("json");
    if !matches!(format, "json" | "text") {
        return Err(ApiError::InvalidRequest(
            "wake_format must be 'json' or 'text'".into(),
        ));
    }
    let url = wake_urls::check_url(state, url)?;
    let secret = wake_urls::new_secret();
    sqlx::query!(
        r#"
        INSERT INTO agent_wake (agent_id, url, format, secret)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (agent_id) DO UPDATE
           SET url = EXCLUDED.url, format = EXCLUDED.format, secret = EXCLUDED.secret,
               enabled = true, failures = 0, last_error = NULL, disabled_at = NULL,
               updated_at = now()
        "#,
        id,
        url,
        format,
        secret,
    )
    .execute(&state.db)
    .await?;
    state.heartbeats.set_wake(id, true);
    let mut body = read(state, id).await?.ok_or(ApiError::NotFound)?;
    body["secret"] = json!(secret);
    Ok(body)
}

/// Clear agent `id`'s wake URL. Shared with MCP `update_agent`.
pub(crate) async fn clear_wake(state: &RelayState, user: &AuthUser, id: Uuid) -> ApiResult<()> {
    manageable(state, user, id).await?;
    sqlx::query!("DELETE FROM agent_wake WHERE agent_id = $1", id)
        .execute(&state.db)
        .await?;
    state.heartbeats.set_wake(id, false);
    Ok(())
}

/// The wake URL as its owners see it, without the secret.
pub(crate) async fn read(state: &RelayState, id: Uuid) -> ApiResult<Option<Value>> {
    let row = sqlx::query!(
        r#"
        SELECT url, format, enabled, failures, last_sent_at, last_ok_at, last_error,
               disabled_at, created_at, updated_at
          FROM agent_wake WHERE agent_id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|r| {
        json!({
            "url": r.url,
            "format": r.format,
            "enabled": r.enabled,
            "failures": r.failures,
            "last_sent_at": r.last_sent_at,
            "last_ok_at": r.last_ok_at,
            "last_error": r.last_error,
            "disabled_at": r.disabled_at,
            "created_at": r.created_at,
            "updated_at": r.updated_at,
        })
    }))
}
