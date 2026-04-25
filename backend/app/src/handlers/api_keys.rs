use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{generate_api_key, hash_api_key, key_prefix, AuthUser};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ApiKeyDto {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub account_id: Option<Uuid>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    /// Optional scope — if set, the key only authenticates inside this account.
    pub account_id: Option<Uuid>,
    /// TTL in days. Null = never expires.
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub api_key: ApiKeyDto,
    /// Plaintext — shown to the user exactly once. Never stored.
    pub plaintext: String,
}

// ─────────────────────────────────────────────────────────
// GET /v1/api-keys
// ─────────────────────────────────────────────────────────
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<ApiKeyDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, key_prefix, account_id, last_used_at, expires_at, revoked_at, created_at
        FROM api_keys
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| ApiKeyDto {
                id: r.id,
                name: r.name,
                prefix: r.key_prefix,
                account_id: r.account_id,
                last_used_at: r.last_used_at,
                expires_at: r.expires_at,
                revoked_at: r.revoked_at,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

// ─────────────────────────────────────────────────────────
// POST /v1/api-keys
// ─────────────────────────────────────────────────────────
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> ApiResult<Json<CreateApiKeyResponse>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }
    if let Some(d) = req.expires_in_days {
        if !(1..=3650).contains(&d) {
            return Err(ApiError::InvalidRequest(
                "expires_in_days must be between 1 and 3650".into(),
            ));
        }
    }

    // If a scope account is requested, verify the user is in it.
    if let Some(account_id) = req.account_id {
        let ok = sqlx::query!(
            r#"SELECT 1 as one FROM account_memberships WHERE user_id = $1 AND account_id = $2 LIMIT 1"#,
            user.user_id,
            account_id,
        )
        .fetch_optional(&state.db)
        .await?
        .is_some();
        if !ok {
            return Err(ApiError::Forbidden);
        }
    }

    let plaintext = generate_api_key();
    let key_hash = hash_api_key(&plaintext);
    let prefix = key_prefix(&plaintext);
    let id = Uuid::now_v7();
    let expires_at = req.expires_in_days.map(|d| Utc::now() + Duration::days(d));

    let inserted = sqlx::query!(
        r#"
        INSERT INTO api_keys (id, user_id, account_id, name, key_hash, key_prefix, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, name, key_prefix, account_id, last_used_at, expires_at, revoked_at, created_at
        "#,
        id,
        user.user_id,
        req.account_id,
        req.name,
        key_hash,
        prefix,
        expires_at,
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(CreateApiKeyResponse {
        api_key: ApiKeyDto {
            id: inserted.id,
            name: inserted.name,
            prefix: inserted.key_prefix,
            account_id: inserted.account_id,
            last_used_at: inserted.last_used_at,
            expires_at: inserted.expires_at,
            revoked_at: inserted.revoked_at,
            created_at: inserted.created_at,
        },
        plaintext,
    }))
}

// ─────────────────────────────────────────────────────────
// DELETE /v1/api-keys/:id — revoke
// ─────────────────────────────────────────────────────────
pub async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let result = sqlx::query!(
        r#"
        UPDATE api_keys
        SET revoked_at = now()
        WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
        "#,
        id,
        user.user_id,
    )
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}
