//! Capability CRUD nested under an agent.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

#[derive(Debug, Serialize)]
pub struct CapabilityDto {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub visibility: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_schema")]
    pub input_schema: serde_json::Value,
    #[serde(default = "default_schema")]
    pub output_schema: serde_json::Value,
    #[serde(default)]
    pub visibility: Option<String>,
}

fn default_schema() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
    pub visibility: Option<String>,
}

async fn agent_account_for_member(
    state: &RelayState,
    user_id: Uuid,
    agent_id: Uuid,
) -> Result<Uuid, ApiError> {
    let row = sqlx::query!(
        r#"SELECT account_id, visibility FROM agents WHERE id = $1"#,
        agent_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !user_is_member(&state.db, user_id, row.account_id).await? {
        // Hide existence from non-members of private agents.
        if row.visibility != "network" {
            return Err(ApiError::NotFound);
        }
        return Err(ApiError::Forbidden);
    }
    Ok(row.account_id)
}

// ─── GET /v1/agents/{id}/capabilities ────────────────────
pub async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(agent_id): Path<Uuid>,
) -> ApiResult<Json<Vec<CapabilityDto>>> {
    // Members see all; non-members see only network-visible capabilities of network agents.
    let agent = sqlx::query!(
        r#"SELECT account_id, visibility FROM agents WHERE id = $1"#,
        agent_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let is_member = user_is_member(&state.db, user.user_id, agent.account_id).await?;
    if !is_member && agent.visibility != "network" {
        return Err(ApiError::NotFound);
    }

    // One query — non-members get only network-visible rows.
    let rows = sqlx::query!(
        r#"
        SELECT id, agent_id, name, description, input_schema, output_schema,
               visibility, created_at, updated_at
        FROM agent_capabilities
        WHERE agent_id = $1
          AND ($2::boolean OR visibility = 'network')
        ORDER BY name ASC
        "#,
        agent_id,
        is_member,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| CapabilityDto {
                id: r.id,
                agent_id: r.agent_id,
                name: r.name,
                description: r.description,
                input_schema: r.input_schema,
                output_schema: r.output_schema,
                visibility: r.visibility,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect(),
    ))
}

// ─── POST /v1/agents/{id}/capabilities ───────────────────
pub async fn create(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(agent_id): Path<Uuid>,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<CapabilityDto>> {
    agent_account_for_member(&state, user.user_id, agent_id).await?;

    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
        return Err(ApiError::InvalidRequest(
            "name must be ascii alphanumeric, underscore, or dot".into(),
        ));
    }
    let visibility = req.visibility.as_deref().unwrap_or("network");
    if !matches!(visibility, "private" | "network") {
        return Err(ApiError::InvalidRequest("visibility must be private|network".into()));
    }

    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agent_capabilities
            (id, agent_id, name, description, input_schema, output_schema, visibility)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (agent_id, name) DO NOTHING
        RETURNING id, agent_id, name, description, input_schema, output_schema,
                  visibility, created_at, updated_at
        "#,
        id,
        agent_id,
        name,
        req.description,
        req.input_schema,
        req.output_schema,
        visibility,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::Conflict(format!("capability '{name}' already exists for this agent")))?;

    Ok(Json(CapabilityDto {
        id: inserted.id,
        agent_id: inserted.agent_id,
        name: inserted.name,
        description: inserted.description,
        input_schema: inserted.input_schema,
        output_schema: inserted.output_schema,
        visibility: inserted.visibility,
        created_at: inserted.created_at,
        updated_at: inserted.updated_at,
    }))
}

// ─── PATCH /v1/agents/{id}/capabilities/{cap_id} ─────────
pub async fn update(
    State(state): State<RelayState>,
    user: AuthUser,
    Path((agent_id, cap_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<Json<CapabilityDto>> {
    agent_account_for_member(&state, user.user_id, agent_id).await?;

    if let Some(v) = req.visibility.as_deref() {
        if !matches!(v, "private" | "network") {
            return Err(ApiError::InvalidRequest("visibility must be private|network".into()));
        }
    }

    let updated = sqlx::query!(
        r#"
        UPDATE agent_capabilities
        SET description = COALESCE($3, description),
            input_schema = COALESCE($4, input_schema),
            output_schema = COALESCE($5, output_schema),
            visibility = COALESCE($6, visibility)
        WHERE id = $1 AND agent_id = $2
        RETURNING id, agent_id, name, description, input_schema, output_schema,
                  visibility, created_at, updated_at
        "#,
        cap_id,
        agent_id,
        req.description.as_deref(),
        req.input_schema,
        req.output_schema,
        req.visibility.as_deref(),
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(CapabilityDto {
        id: updated.id,
        agent_id: updated.agent_id,
        name: updated.name,
        description: updated.description,
        input_schema: updated.input_schema,
        output_schema: updated.output_schema,
        visibility: updated.visibility,
        created_at: updated.created_at,
        updated_at: updated.updated_at,
    }))
}

// ─── DELETE /v1/agents/{id}/capabilities/{cap_id} ────────
pub async fn delete(
    State(state): State<RelayState>,
    user: AuthUser,
    Path((agent_id, cap_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    agent_account_for_member(&state, user.user_id, agent_id).await?;

    let res = sqlx::query!(
        r#"DELETE FROM agent_capabilities WHERE id = $1 AND agent_id = $2"#,
        cap_id,
        agent_id
    )
    .execute(&state.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
