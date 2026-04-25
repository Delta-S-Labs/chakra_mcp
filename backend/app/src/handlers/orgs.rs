use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct OrgDto {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub account_type: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrgRequest {
    pub slug: String,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct MemberDto {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub email: String,
    pub role: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InviteDto {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    /// One-shot — only returned at creation, never stored in plaintext.
    pub token: String,
}

// ─────────────────────────────────────────────────────────
// GET /v1/orgs — list orgs the current user belongs to
// ─────────────────────────────────────────────────────────
pub async fn list(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<Vec<OrgDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT a.id, a.slug, a.display_name, a.account_type, m.role, a.created_at
        FROM account_memberships m
        JOIN accounts a ON a.id = m.account_id
        WHERE m.user_id = $1
        ORDER BY a.account_type DESC, a.created_at ASC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| OrgDto {
                id: r.id,
                slug: r.slug,
                display_name: r.display_name,
                account_type: r.account_type,
                role: r.role,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

// ─────────────────────────────────────────────────────────
// POST /v1/orgs — create a new organization
// ─────────────────────────────────────────────────────────
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateOrgRequest>,
) -> ApiResult<Json<OrgDto>> {
    let slug = req.slug.trim();
    if slug.is_empty() {
        return Err(ApiError::InvalidRequest("slug is required".into()));
    }
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ApiError::InvalidRequest(
            "slug may only contain a-z, 0-9, hyphens, underscores".into(),
        ));
    }
    if req.display_name.trim().is_empty() {
        return Err(ApiError::InvalidRequest("display_name is required".into()));
    }

    let mut tx = state.db.begin().await?;

    let exists = sqlx::query!(r#"SELECT 1 as one FROM accounts WHERE slug = $1"#, slug)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_some() {
        return Err(ApiError::Conflict(format!("org slug '{slug}' is taken")));
    }

    let account_id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
        VALUES ($1, $2, $3, 'organization', $4)
        RETURNING id, slug, display_name, account_type, created_at
        "#,
        account_id,
        slug,
        req.display_name,
        user.user_id,
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO account_memberships (id, account_id, user_id, role)
        VALUES ($1, $2, $3, 'owner')
        "#,
        Uuid::now_v7(),
        account_id,
        user.user_id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(OrgDto {
        id: inserted.id,
        slug: inserted.slug,
        display_name: inserted.display_name,
        account_type: inserted.account_type,
        role: "owner".into(),
        created_at: inserted.created_at,
    }))
}

// ─────────────────────────────────────────────────────────
// GET /v1/orgs/:slug
// ─────────────────────────────────────────────────────────
pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<OrgDto>> {
    let row = sqlx::query!(
        r#"
        SELECT a.id, a.slug, a.display_name, a.account_type, m.role, a.created_at
        FROM accounts a
        JOIN account_memberships m ON m.account_id = a.id
        WHERE a.slug = $1 AND m.user_id = $2
        LIMIT 1
        "#,
        slug,
        user.user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(OrgDto {
        id: row.id,
        slug: row.slug,
        display_name: row.display_name,
        account_type: row.account_type,
        role: row.role,
        created_at: row.created_at,
    }))
}

// ─────────────────────────────────────────────────────────
// GET /v1/orgs/:slug/members
// ─────────────────────────────────────────────────────────
pub async fn list_members(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<MemberDto>>> {
    // Membership check inline.
    let account = sqlx::query!(
        r#"
        SELECT a.id
        FROM accounts a
        JOIN account_memberships m ON m.account_id = a.id
        WHERE a.slug = $1 AND m.user_id = $2
        LIMIT 1
        "#,
        slug,
        user.user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let members = sqlx::query!(
        r#"
        SELECT u.id as user_id, u.email, u.display_name, u.avatar_url, m.role, m.joined_at
        FROM account_memberships m
        JOIN users u ON u.id = m.user_id
        WHERE m.account_id = $1
        ORDER BY m.joined_at ASC
        "#,
        account.id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        members
            .into_iter()
            .map(|r| MemberDto {
                user_id: r.user_id,
                email: r.email,
                display_name: r.display_name,
                avatar_url: r.avatar_url,
                role: r.role,
                joined_at: r.joined_at,
            })
            .collect(),
    ))
}

// ─────────────────────────────────────────────────────────
// POST /v1/orgs/:slug/invites — create an invite token
//
// MVP: only the org owner/admin can invite; the token is returned once
// in the response and email delivery is a TODO. The frontend should
// surface the link for the inviter to copy.
// ─────────────────────────────────────────────────────────
pub async fn create_invite(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
    Json(req): Json<CreateInviteRequest>,
) -> ApiResult<Json<InviteDto>> {
    let role = req.role.unwrap_or_else(|| "member".to_string());
    if !matches!(role.as_str(), "owner" | "admin" | "member") {
        return Err(ApiError::InvalidRequest("role must be owner|admin|member".into()));
    }
    if req.email.trim().is_empty() {
        return Err(ApiError::InvalidRequest("email is required".into()));
    }

    let access = sqlx::query!(
        r#"
        SELECT a.id, m.role
        FROM accounts a
        JOIN account_memberships m ON m.account_id = a.id
        WHERE a.slug = $1 AND m.user_id = $2
        LIMIT 1
        "#,
        slug,
        user.user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !matches!(access.role.as_str(), "owner" | "admin") {
        return Err(ApiError::Forbidden);
    }

    // Generate a one-shot invite token; store only its hash.
    let token = format!("inv_{}", uuid::Uuid::now_v7().simple());
    let token_hash = {
        let mut h = Sha256::new();
        h.update(token.as_bytes());
        hex::encode(h.finalize())
    };
    let invite_id = Uuid::now_v7();
    let expires_at = Utc::now() + Duration::days(7);

    sqlx::query!(
        r#"
        INSERT INTO account_invites (id, account_id, email, role, invited_by, token_hash, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (account_id, email) DO UPDATE SET
            role = EXCLUDED.role,
            token_hash = EXCLUDED.token_hash,
            expires_at = EXCLUDED.expires_at,
            invited_by = EXCLUDED.invited_by,
            accepted_at = NULL
        "#,
        invite_id,
        access.id,
        req.email,
        role,
        user.user_id,
        token_hash,
        expires_at,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(InviteDto {
        id: invite_id,
        email: req.email,
        role,
        expires_at,
        token,
    }))
}
