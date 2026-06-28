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
    /// True when this capability is callable by any registered agent
    /// without a friendship/grant. Owner opt-in via the capability
    /// create/update endpoints; default false. See migration 0022 +
    /// docs/superpowers/specs/...-public-invokable-...
    pub public_invoke: bool,
    /// Per-invoker monthly cap (calendar month) for a public
    /// capability. None when `public_invoke` is false; required (>=1)
    /// when true.
    pub public_monthly_quota_per_agent: Option<i32>,
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
    /// HITL gate (issue #69 PR 2). `"autonomous"` (default) or
    /// `"human_in_loop"`. The relay's
    /// `POST /v1/invocations/{id}/result` enforces the value — see
    /// `handlers::invoke::report_result`.
    #[serde(default)]
    pub semantics: Option<String>,
    /// Opt this capability into being callable by any registered
    /// agent without a friendship. Default false. When true,
    /// `visibility` must resolve to `network` and
    /// `public_monthly_quota_per_agent` must be >= 1 (DB CHECKs
    /// `cap_public_requires_network` + `cap_public_requires_quota`).
    #[serde(default)]
    pub public_invoke: Option<bool>,
    /// Owner-set per-invoker monthly cap. Required when
    /// `public_invoke` is true. Counted against `relay_invocations`
    /// rows in the current calendar month.
    #[serde(default)]
    pub public_monthly_quota_per_agent: Option<i32>,
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
    /// COALESCE: absent keys leave the column unchanged. When set so
    /// that effective `public_invoke=true`, the effective `visibility`
    /// must be `network` and the effective monthly quota must be >= 1
    /// — server pre-validates by reading the current row.
    pub public_invoke: Option<bool>,
    pub public_monthly_quota_per_agent: Option<i32>,
}

/// Pre-validates the post-write public-invoke triple before letting
/// it hit the DB CHECKs. Returns 400 with a friendly message rather
/// than letting a constraint violation bubble up as a 500.
fn validate_public_invoke_combo(
    public_invoke: bool,
    visibility: &str,
    quota: Option<i32>,
) -> Result<(), ApiError> {
    if !public_invoke {
        return Ok(());
    }
    if visibility != "network" {
        return Err(ApiError::InvalidRequest(
            "public_invoke=true requires visibility='network'".into(),
        ));
    }
    match quota {
        Some(n) if n >= 1 => Ok(()),
        Some(_) => Err(ApiError::InvalidRequest(
            "public_monthly_quota_per_agent must be >= 1 when public_invoke=true".into(),
        )),
        None => Err(ApiError::InvalidRequest(
            "public_monthly_quota_per_agent is required when public_invoke=true".into(),
        )),
    }
}

async fn agent_account_for_member(
    state: &RelayState,
    user: &AuthUser,
    agent_id: Uuid,
) -> Result<Uuid, ApiError> {
    let row = sqlx::query!(
        r#"SELECT account_id, visibility FROM agents WHERE id = $1"#,
        agent_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !user_is_member(&state.db, user.user_id, row.account_id).await? {
        // Hide existence from non-members of private agents.
        if row.visibility != "network" {
            return Err(ApiError::NotFound);
        }
        return Err(ApiError::Forbidden);
    }
    // Scope gate (migration 0027): the credential must be allowed to manage
    // this agent, not merely belong to its account. Capability create /
    // update / delete all funnel through here, so this one check covers
    // every capability write.
    let grant = crate::auth::resolve_grant(&state.db, user).await?;
    if !crate::auth::grant_allows_agent(&state.db, &grant, agent_id).await? {
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
               visibility, created_at, updated_at,
               public_invoke, public_monthly_quota_per_agent
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
                public_invoke: r.public_invoke,
                public_monthly_quota_per_agent: r.public_monthly_quota_per_agent,
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
    let acct = agent_account_for_member(&state, &user, agent_id).await?;

    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(ApiError::InvalidRequest(
            "name must be ascii alphanumeric, underscore, or dot".into(),
        ));
    }
    let visibility = req.visibility.as_deref().unwrap_or("network");
    if !matches!(visibility, "private" | "network") {
        return Err(ApiError::InvalidRequest(
            "visibility must be private|network".into(),
        ));
    }
    // The DB CHECK constraint (migration 0020) would also reject an
    // invalid value, but pre-validating gives a 400 with a friendly
    // message instead of a 500 from a constraint violation.
    let semantics = req.semantics.as_deref().unwrap_or("autonomous");
    if !matches!(semantics, "autonomous" | "human_in_loop") {
        return Err(ApiError::InvalidRequest(
            "semantics must be autonomous|human_in_loop".into(),
        ));
    }

    let public_invoke = req.public_invoke.unwrap_or(false);
    validate_public_invoke_combo(
        public_invoke,
        visibility,
        req.public_monthly_quota_per_agent,
    )?;
    // When `public_invoke` is false the quota column is meaningless;
    // we store NULL so a later flip to true forces the owner to set a
    // fresh quota rather than reusing a stale leftover value.
    let public_quota = if public_invoke {
        req.public_monthly_quota_per_agent
    } else {
        None
    };

    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agent_capabilities
            (id, agent_id, name, description, input_schema, output_schema,
             visibility, semantics, created_by_user_id,
             public_invoke, public_monthly_quota_per_agent)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (agent_id, name) DO NOTHING
        RETURNING id, agent_id, name, description, input_schema, output_schema,
                  visibility, created_at, updated_at,
                  public_invoke, public_monthly_quota_per_agent
        "#,
        id,
        agent_id,
        name,
        req.description,
        req.input_schema,
        req.output_schema,
        visibility,
        semantics,
        user.user_id,
        public_invoke,
        public_quota,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        ApiError::Conflict(format!("capability '{name}' already exists for this agent"))
    })?;

    crate::events::record_audit(
        &state.db,
        &user,
        Some(acct),
        "capability.create",
        "capability",
        Some(inserted.id),
        Some(agent_id),
        &format!("published capability {}", inserted.name),
        serde_json::json!({ "name": inserted.name, "visibility": inserted.visibility }),
    )
    .await;

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
        public_invoke: inserted.public_invoke,
        public_monthly_quota_per_agent: inserted.public_monthly_quota_per_agent,
    }))
}

// ─── PATCH /v1/agents/{id}/capabilities/{cap_id} ─────────
pub async fn update(
    State(state): State<RelayState>,
    user: AuthUser,
    Path((agent_id, cap_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<Json<CapabilityDto>> {
    let acct = agent_account_for_member(&state, &user, agent_id).await?;

    if let Some(v) = req.visibility.as_deref() {
        if !matches!(v, "private" | "network") {
            return Err(ApiError::InvalidRequest(
                "visibility must be private|network".into(),
            ));
        }
    }

    // Read the pre-update row so we can pre-validate the effective
    // (COALESCE'd) public_invoke / visibility / quota triple before
    // letting the UPDATE hit the DB CHECKs. One extra round-trip in
    // exchange for a friendly 400 instead of a 500-shaped constraint
    // violation.
    let current = sqlx::query!(
        r#"
        SELECT visibility, public_invoke, public_monthly_quota_per_agent
        FROM agent_capabilities
        WHERE id = $1 AND agent_id = $2
        "#,
        cap_id,
        agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let effective_visibility: &str = req
        .visibility
        .as_deref()
        .unwrap_or(current.visibility.as_str());
    let effective_public_invoke = req.public_invoke.unwrap_or(current.public_invoke);
    // When the PATCH explicitly sets `public_invoke=false`, we wipe the
    // stale quota so re-enabling later forces a fresh choice. Otherwise
    // we COALESCE the new value in or leave the existing quota.
    let effective_quota: Option<i32> = if !effective_public_invoke {
        None
    } else {
        req.public_monthly_quota_per_agent
            .or(current.public_monthly_quota_per_agent)
    };
    validate_public_invoke_combo(
        effective_public_invoke,
        effective_visibility,
        effective_quota,
    )?;

    let updated = sqlx::query!(
        r#"
        UPDATE agent_capabilities
        SET description = COALESCE($3, description),
            input_schema = COALESCE($4, input_schema),
            output_schema = COALESCE($5, output_schema),
            visibility = COALESCE($6, visibility),
            public_invoke = COALESCE($7, public_invoke),
            public_monthly_quota_per_agent = $8
        WHERE id = $1 AND agent_id = $2
        RETURNING id, agent_id, name, description, input_schema, output_schema,
                  visibility, created_at, updated_at,
                  public_invoke, public_monthly_quota_per_agent
        "#,
        cap_id,
        agent_id,
        req.description.as_deref(),
        req.input_schema,
        req.output_schema,
        req.visibility.as_deref(),
        req.public_invoke,
        effective_quota,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    crate::events::record_audit(
        &state.db,
        &user,
        Some(acct),
        "capability.update",
        "capability",
        Some(updated.id),
        Some(agent_id),
        &format!("updated capability {}", updated.name),
        serde_json::json!({ "name": updated.name, "visibility": updated.visibility }),
    )
    .await;

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
        public_invoke: updated.public_invoke,
        public_monthly_quota_per_agent: updated.public_monthly_quota_per_agent,
    }))
}

// ─── DELETE /v1/agents/{id}/capabilities/{cap_id} ────────
pub async fn delete(
    State(state): State<RelayState>,
    user: AuthUser,
    Path((agent_id, cap_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let acct = agent_account_for_member(&state, &user, agent_id).await?;

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

    crate::events::record_audit(
        &state.db,
        &user,
        Some(acct),
        "capability.delete",
        "capability",
        Some(cap_id),
        Some(agent_id),
        "deleted capability",
        serde_json::json!({}),
    )
    .await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
