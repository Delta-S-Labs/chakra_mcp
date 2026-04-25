use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::ApiResult;

use crate::auth::AdminUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct AdminUserDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AdminOrgDto {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub account_type: String,
    pub member_count: i64,
    pub owner_email: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AdminApiKeyDto {
    pub id: Uuid,
    pub user_email: String,
    pub name: String,
    pub prefix: String,
    pub account_id: Option<Uuid>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────
// GET /v1/admin/users
// ─────────────────────────────────────────────────────────
pub async fn list_users(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<AdminUserDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, email, display_name, avatar_url, is_admin, created_at
        FROM users
        ORDER BY created_at DESC
        "#
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AdminUserDto {
                id: r.id,
                email: r.email,
                display_name: r.display_name,
                avatar_url: r.avatar_url,
                is_admin: r.is_admin,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

// ─────────────────────────────────────────────────────────
// GET /v1/admin/orgs
// ─────────────────────────────────────────────────────────
pub async fn list_orgs(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<AdminOrgDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT
          a.id,
          a.slug,
          a.display_name,
          a.account_type,
          a.created_at,
          (SELECT COUNT(*) FROM account_memberships m WHERE m.account_id = a.id) as "member_count!",
          (SELECT u.email FROM users u WHERE u.id = a.owner_user_id) as owner_email
        FROM accounts a
        ORDER BY a.created_at DESC
        "#
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AdminOrgDto {
                id: r.id,
                slug: r.slug,
                display_name: r.display_name,
                account_type: r.account_type,
                member_count: r.member_count,
                owner_email: r.owner_email,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

// ─────────────────────────────────────────────────────────
// GET /v1/admin/api-keys
// ─────────────────────────────────────────────────────────
pub async fn list_api_keys(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<AdminApiKeyDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT
          k.id,
          u.email as user_email,
          k.name,
          k.key_prefix,
          k.account_id,
          k.last_used_at,
          k.expires_at,
          k.revoked_at,
          k.created_at
        FROM api_keys k
        JOIN users u ON u.id = k.user_id
        ORDER BY k.created_at DESC
        "#
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AdminApiKeyDto {
                id: r.id,
                user_email: r.user_email,
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
