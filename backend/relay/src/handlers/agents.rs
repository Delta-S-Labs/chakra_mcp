//! Agent CRUD + network discovery.
//!
//! An agent always lives inside an account (personal or organization).
//! Mutations require the caller to be a member of that account; admin
//! mutations (visibility flips, deletes) require owner|admin role.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_can_admin_account, user_is_member, AuthUser};
use crate::state::RelayState;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AgentDto {
    pub id: Uuid,
    pub account_id: Uuid,
    pub account_slug: String,
    pub account_display_name: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub visibility: String,
    pub endpoint_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// True when the requesting user is a member of the owning account.
    pub is_mine: bool,
    pub capability_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub account_id: Uuid,
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub visibility: Option<String>,
    pub endpoint_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<String>,
    pub endpoint_url: Option<Option<String>>,
}

// ─── GET /v1/agents — list mine ──────────────────────────
pub async fn list_mine(
    State(state): State<RelayState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<AgentDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.account_id IN (
            SELECT account_id FROM account_memberships WHERE user_id = $1
        )
        ORDER BY a.created_at DESC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AgentDto {
                id: r.id,
                account_id: r.account_id,
                account_slug: r.account_slug,
                account_display_name: r.account_display_name,
                slug: r.slug,
                display_name: r.display_name,
                description: r.description,
                visibility: r.visibility,
                endpoint_url: r.endpoint_url,
                created_at: r.created_at,
                updated_at: r.updated_at,
                is_mine: true,
                capability_count: r.capability_count,
            })
            .collect(),
    ))
}

// ─── GET /v1/network/agents — discover ───────────────────
pub async fn list_network(
    State(state): State<RelayState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<AgentDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c
                WHERE c.agent_id = a.id AND c.visibility = 'network') as "capability_count!",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.account_id = a.account_id AND m.user_id = $1
            ) as "is_mine!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.visibility = 'network'
        ORDER BY a.created_at DESC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AgentDto {
                id: r.id,
                account_id: r.account_id,
                account_slug: r.account_slug,
                account_display_name: r.account_display_name,
                slug: r.slug,
                display_name: r.display_name,
                description: r.description,
                visibility: r.visibility,
                endpoint_url: if r.is_mine { r.endpoint_url } else { None },
                created_at: r.created_at,
                updated_at: r.updated_at,
                is_mine: r.is_mine,
                capability_count: r.capability_count,
            })
            .collect(),
    ))
}

// ─── GET /v1/agents/{id} ─────────────────────────────────
pub async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AgentDto>> {
    let r = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.account_id = a.account_id AND m.user_id = $1
            ) as "is_mine!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.id = $2
        "#,
        user.user_id,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !r.is_mine && r.visibility != "network" {
        return Err(ApiError::NotFound);
    }

    Ok(Json(AgentDto {
        id: r.id,
        account_id: r.account_id,
        account_slug: r.account_slug,
        account_display_name: r.account_display_name,
        slug: r.slug,
        display_name: r.display_name,
        description: r.description,
        visibility: r.visibility,
        endpoint_url: if r.is_mine { r.endpoint_url } else { None },
        created_at: r.created_at,
        updated_at: r.updated_at,
        is_mine: r.is_mine,
        capability_count: r.capability_count,
    }))
}

// ─── POST /v1/agents ─────────────────────────────────────
pub async fn create(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<AgentDto>> {
    let slug = req.slug.trim().to_lowercase();
    let display_name = req.display_name.trim().to_string();
    if slug.is_empty() || display_name.is_empty() {
        return Err(ApiError::InvalidRequest("slug and display_name are required".into()));
    }
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ApiError::InvalidRequest(
            "slug must be ascii alphanumeric, hyphen, or underscore".into(),
        ));
    }
    let visibility = req.visibility.as_deref().unwrap_or("private");
    if !matches!(visibility, "private" | "network") {
        return Err(ApiError::InvalidRequest("visibility must be private|network".into()));
    }

    if !user_is_member(&state.db, user.user_id, req.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    let id = Uuid::now_v7();
    // ON CONFLICT must match the partial unique index (`tombstoned_at IS NULL`)
    // introduced by migration 0009 — old code targeted a full UNIQUE that was
    // replaced. The predicate keeps the conflict scope to live (non-tombstoned)
    // rows, which is exactly the semantics we want: re-registering after a
    // tombstone is a separate explicit "untombstone" action, not an implicit
    // upsert.
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agents (id, account_id, created_by_user_id, slug, display_name, description, visibility, endpoint_url)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (account_id, slug) WHERE tombstoned_at IS NULL DO NOTHING
        RETURNING id, account_id, slug, display_name, description, visibility, endpoint_url, created_at, updated_at
        "#,
        id,
        req.account_id,
        user.user_id,
        slug,
        display_name,
        req.description,
        visibility,
        req.endpoint_url,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::Conflict(format!("agent slug '{slug}' already exists in this account")))?;

    let acc = sqlx::query!(
        r#"SELECT slug, display_name FROM accounts WHERE id = $1"#,
        inserted.account_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AgentDto {
        id: inserted.id,
        account_id: inserted.account_id,
        account_slug: acc.slug,
        account_display_name: acc.display_name,
        slug: inserted.slug,
        display_name: inserted.display_name,
        description: inserted.description,
        visibility: inserted.visibility,
        endpoint_url: inserted.endpoint_url,
        created_at: inserted.created_at,
        updated_at: inserted.updated_at,
        is_mine: true,
        capability_count: 0,
    }))
}

// ─── PATCH /v1/agents/{id} ───────────────────────────────
pub async fn update(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<Json<AgentDto>> {
    let row = sqlx::query!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !user_is_member(&state.db, user.user_id, row.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    if let Some(v) = req.visibility.as_deref() {
        if !matches!(v, "private" | "network") {
            return Err(ApiError::InvalidRequest("visibility must be private|network".into()));
        }
        if v == "network" && !user_can_admin_account(&state.db, user.user_id, row.account_id).await? {
            return Err(ApiError::Forbidden);
        }
    }

    sqlx::query!(
        r#"
        UPDATE agents
        SET display_name = COALESCE($2, display_name),
            description = COALESCE($3, description),
            visibility = COALESCE($4, visibility),
            endpoint_url = CASE
                WHEN $5::boolean THEN $6
                ELSE endpoint_url
            END
        WHERE id = $1
        "#,
        id,
        req.display_name.as_deref(),
        req.description.as_deref(),
        req.visibility.as_deref(),
        req.endpoint_url.is_some(),
        req.endpoint_url.flatten(),
    )
    .execute(&state.db)
    .await?;

    let r = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.id = $1
        "#,
        id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AgentDto {
        id: r.id,
        account_id: r.account_id,
        account_slug: r.account_slug,
        account_display_name: r.account_display_name,
        slug: r.slug,
        display_name: r.display_name,
        description: r.description,
        visibility: r.visibility,
        endpoint_url: r.endpoint_url,
        created_at: r.created_at,
        updated_at: r.updated_at,
        is_mine: true,
        capability_count: r.capability_count,
    }))
}

// ─── DELETE /v1/agents/{id} ──────────────────────────────
pub async fn delete(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let row = sqlx::query!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !user_can_admin_account(&state.db, user.user_id, row.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    sqlx::query!(r#"DELETE FROM agents WHERE id = $1"#, id)
        .execute(&state.db)
        .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
