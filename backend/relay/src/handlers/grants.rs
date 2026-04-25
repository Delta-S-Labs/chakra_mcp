//! Grants — directional capability access between agents.
//!
//! Creating a grant requires:
//!   * caller is a member of the granter agent's account
//!   * the named capability belongs to the granter agent
//!   * an accepted friendship exists between granter and grantee agents
//!     in either direction
//!
//! Revoking a grant requires caller to be a member of the granter
//! agent's account. Revocation is permanent for that row; to re-grant,
//! create a new one.

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::handlers::friendships::AgentSummary;
use crate::state::RelayState;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct GrantDto {
    pub id: Uuid,
    pub status: String,
    pub granter: AgentSummary,
    pub grantee: AgentSummary,
    pub capability_id: Uuid,
    pub capability_name: String,
    pub capability_visibility: String,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoke_reason: Option<String>,
    /// True when the requesting user is on the granter side.
    pub i_granted: bool,
    /// True when the requesting user is on the grantee side.
    pub i_received: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub granter_agent_id: Uuid,
    pub grantee_agent_id: Uuid,
    pub capability_id: Uuid,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RevokeRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// "outbound" (mine, as granter), "inbound" (as grantee), default both.
    pub direction: Option<String>,
    pub status: Option<String>,
}

// ─── Helpers ─────────────────────────────────────────────

async fn fetch_grant(db: &PgPool, user_id: Uuid, id: Uuid) -> Result<GrantDto, ApiError> {
    let r = sqlx::query!(
        r#"
        SELECT
            g.id, g.status, g.capability_id,
            g.granted_at, g.expires_at, g.revoked_at, g.revoke_reason,
            ga.id   as g_agent_id,   ga.slug as g_agent_slug,   ga.display_name as g_agent_display_name,
            gacc.id as g_acct_id,    gacc.slug as g_acct_slug,  gacc.display_name as g_acct_display_name,
            ea.id   as e_agent_id,   ea.slug as e_agent_slug,   ea.display_name as e_agent_display_name,
            eacc.id as e_acct_id,    eacc.slug as e_acct_slug,  eacc.display_name as e_acct_display_name,
            cap.name as capability_name, cap.visibility as capability_visibility,
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ga.account_id AND m.user_id = $2) as "i_granted!",
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ea.account_id AND m.user_id = $2) as "i_received!"
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN accounts gacc ON gacc.id = ga.account_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN accounts eacc ON eacc.id = ea.account_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        WHERE g.id = $1
        "#,
        id,
        user_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !r.i_granted && !r.i_received {
        return Err(ApiError::NotFound);
    }

    Ok(GrantDto {
        id: r.id,
        status: r.status,
        granter: AgentSummary {
            id: r.g_agent_id,
            slug: r.g_agent_slug,
            display_name: r.g_agent_display_name,
            account_id: r.g_acct_id,
            account_slug: r.g_acct_slug,
            account_display_name: r.g_acct_display_name,
        },
        grantee: AgentSummary {
            id: r.e_agent_id,
            slug: r.e_agent_slug,
            display_name: r.e_agent_display_name,
            account_id: r.e_acct_id,
            account_slug: r.e_acct_slug,
            account_display_name: r.e_acct_display_name,
        },
        capability_id: r.capability_id,
        capability_name: r.capability_name,
        capability_visibility: r.capability_visibility,
        granted_at: r.granted_at,
        expires_at: r.expires_at,
        revoked_at: r.revoked_at,
        revoke_reason: r.revoke_reason,
        i_granted: r.i_granted,
        i_received: r.i_received,
    })
}

/// Returns true if granter and grantee have an accepted friendship in
/// either direction. Used as a precondition for issuing a grant.
async fn have_accepted_friendship(
    db: &PgPool,
    a: Uuid,
    b: Uuid,
) -> Result<bool, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT 1 as one FROM friendships
        WHERE status = 'accepted'
          AND (
              (proposer_agent_id = $1 AND target_agent_id = $2)
           OR (proposer_agent_id = $2 AND target_agent_id = $1)
          )
        LIMIT 1
        "#,
        a,
        b,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

// ─── GET /v1/grants ──────────────────────────────────────
pub async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<GrantDto>>> {
    let direction = q.direction.as_deref().unwrap_or("all");
    if !matches!(direction, "all" | "outbound" | "inbound") {
        return Err(ApiError::InvalidRequest(
            "direction must be all|outbound|inbound".into(),
        ));
    }
    if let Some(s) = q.status.as_deref() {
        if !matches!(s, "active" | "revoked" | "expired") {
            return Err(ApiError::InvalidRequest("invalid status".into()));
        }
    }

    let want_outbound = matches!(direction, "all" | "outbound");
    let want_inbound = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT
            g.id, g.status, g.capability_id,
            g.granted_at, g.expires_at, g.revoked_at, g.revoke_reason,
            ga.id   as g_agent_id,   ga.slug as g_agent_slug,   ga.display_name as g_agent_display_name,
            gacc.id as g_acct_id,    gacc.slug as g_acct_slug,  gacc.display_name as g_acct_display_name,
            ea.id   as e_agent_id,   ea.slug as e_agent_slug,   ea.display_name as e_agent_display_name,
            eacc.id as e_acct_id,    eacc.slug as e_acct_slug,  eacc.display_name as e_acct_display_name,
            cap.name as capability_name, cap.visibility as capability_visibility,
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ga.account_id AND m.user_id = $1) as "i_granted!",
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ea.account_id AND m.user_id = $1) as "i_received!"
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN accounts gacc ON gacc.id = ga.account_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN accounts eacc ON eacc.id = ea.account_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        WHERE
            (
                ($2::boolean AND EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ga.account_id AND m.user_id = $1))
             OR ($3::boolean AND EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ea.account_id AND m.user_id = $1))
            )
            AND ($4::text IS NULL OR g.status = $4)
        ORDER BY g.granted_at DESC
        "#,
        user.user_id,
        want_outbound,
        want_inbound,
        q.status,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| GrantDto {
                id: r.id,
                status: r.status,
                granter: AgentSummary {
                    id: r.g_agent_id,
                    slug: r.g_agent_slug,
                    display_name: r.g_agent_display_name,
                    account_id: r.g_acct_id,
                    account_slug: r.g_acct_slug,
                    account_display_name: r.g_acct_display_name,
                },
                grantee: AgentSummary {
                    id: r.e_agent_id,
                    slug: r.e_agent_slug,
                    display_name: r.e_agent_display_name,
                    account_id: r.e_acct_id,
                    account_slug: r.e_acct_slug,
                    account_display_name: r.e_acct_display_name,
                },
                capability_id: r.capability_id,
                capability_name: r.capability_name,
                capability_visibility: r.capability_visibility,
                granted_at: r.granted_at,
                expires_at: r.expires_at,
                revoked_at: r.revoked_at,
                revoke_reason: r.revoke_reason,
                i_granted: r.i_granted,
                i_received: r.i_received,
            })
            .collect(),
    ))
}

// ─── GET /v1/grants/{id} ─────────────────────────────────
pub async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<GrantDto>> {
    Ok(Json(fetch_grant(&state.db, user.user_id, id).await?))
}

// ─── POST /v1/grants ─────────────────────────────────────
pub async fn create(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<GrantDto>> {
    if req.granter_agent_id == req.grantee_agent_id {
        return Err(ApiError::InvalidRequest(
            "granter and grantee must be different agents".into(),
        ));
    }
    if let Some(ts) = req.expires_at {
        if ts <= Utc::now() {
            return Err(ApiError::InvalidRequest(
                "expires_at must be in the future".into(),
            ));
        }
    }

    // Capability must belong to granter agent.
    let cap = sqlx::query!(
        r#"
        SELECT c.id, c.agent_id, ga.account_id as granter_account_id
        FROM agent_capabilities c
        JOIN agents ga ON ga.id = c.agent_id
        WHERE c.id = $1
        "#,
        req.capability_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if cap.agent_id != req.granter_agent_id {
        return Err(ApiError::InvalidRequest(
            "capability does not belong to the named granter agent".into(),
        ));
    }

    // Caller must be a member of the granter's account.
    if !user_is_member(&state.db, user.user_id, cap.granter_account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Grantee must exist.
    let _grantee_exists = sqlx::query_scalar!(
        r#"SELECT id FROM agents WHERE id = $1"#,
        req.grantee_agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Friendship gate.
    if !have_accepted_friendship(&state.db, req.granter_agent_id, req.grantee_agent_id).await? {
        return Err(ApiError::Conflict(
            "no accepted friendship between these agents — propose one first".into(),
        ));
    }

    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO grants
            (id, granter_agent_id, grantee_agent_id, capability_id,
             granted_by_user_id, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
        id,
        req.granter_agent_id,
        req.grantee_agent_id,
        req.capability_id,
        user.user_id,
        req.expires_at,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::Conflict(
                "an active grant for this triple already exists; revoke first to re-grant".into(),
            )
        }
        other => other.into(),
    })?
    .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("insert returned no row")))?;

    Ok(Json(fetch_grant(&state.db, user.user_id, inserted.id).await?))
}

// ─── POST /v1/grants/{id}/revoke ─────────────────────────
pub async fn revoke(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<RevokeRequest>,
) -> ApiResult<Json<GrantDto>> {
    let row = sqlx::query!(
        r#"
        SELECT g.status, ga.account_id as granter_account_id
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        WHERE g.id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "active" {
        return Err(ApiError::Conflict(format!(
            "grant is {}; only active grants can be revoked",
            row.status
        )));
    }

    if !user_is_member(&state.db, user.user_id, row.granter_account_id).await? {
        return Err(ApiError::Forbidden);
    }

    sqlx::query!(
        r#"
        UPDATE grants
        SET status = 'revoked',
            revoked_at = now(),
            revoked_by_user_id = $2,
            revoke_reason = $3
        WHERE id = $1 AND status = 'active'
        "#,
        id,
        user.user_id,
        req.reason,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(fetch_grant(&state.db, user.user_id, id).await?))
}
