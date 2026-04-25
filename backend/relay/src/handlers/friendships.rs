//! Friendship lifecycle: propose, accept, reject, counter, cancel.
//!
//! The acting user must be a member of the *initiating side's* account
//! for each verb:
//!   * propose / cancel → member of proposer agent's account
//!   * accept / reject / counter → member of target agent's account
//!
//! A counter rejects the original (status='countered') and creates a
//! brand-new proposal in the reverse direction with `counter_of_id`
//! pointing back. The original requester then accepts / rejects /
//! counters that — there's no special "counter accepted" verb.

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AgentSummary {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub account_id: Uuid,
    pub account_slug: String,
    pub account_display_name: String,
}

#[derive(Debug, Serialize)]
pub struct FriendshipDto {
    pub id: Uuid,
    pub status: String,
    pub proposer: AgentSummary,
    pub target: AgentSummary,
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
    pub counter_of_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    /// True when the requesting user is on the proposer side (proposer's account).
    pub i_proposed: bool,
    /// True when the requesting user is on the target side.
    pub i_received: bool,
}

#[derive(Debug, Deserialize)]
pub struct ProposeRequest {
    pub proposer_agent_id: Uuid,
    pub target_agent_id: Uuid,
    pub proposer_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseRequest {
    pub response_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CounterRequest {
    /// Counter-message becomes the new proposer's `proposer_message`.
    pub proposer_message: Option<String>,
    /// Optional rejection reason recorded on the original (now countered) row.
    pub response_message: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// "outbound" (mine), "inbound" (theirs), or omitted for both.
    pub direction: Option<String>,
    /// Filter to a specific status, omitted for all non-terminal + decided.
    pub status: Option<String>,
}

// ─── Helpers ─────────────────────────────────────────────

/// One row joined with proposer + target + their accounts. Returned by
/// every endpoint here so the frontend can render without extra round
/// trips.
async fn fetch_friendship(
    db: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<FriendshipDto, ApiError> {
    let r = sqlx::query!(
        r#"
        SELECT
            f.id, f.status, f.proposer_message, f.response_message,
            f.counter_of_id, f.created_at, f.updated_at, f.decided_at,
            pa.id   as p_agent_id,   pa.slug as p_agent_slug,   pa.display_name as p_agent_display_name,
            pacc.id as p_acct_id,    pacc.slug as p_acct_slug,  pacc.display_name as p_acct_display_name,
            ta.id   as t_agent_id,   ta.slug as t_agent_slug,   ta.display_name as t_agent_display_name,
            tacc.id as t_acct_id,    tacc.slug as t_acct_slug,  tacc.display_name as t_acct_display_name,
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = pa.account_id AND m.user_id = $2) as "i_proposed!",
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ta.account_id AND m.user_id = $2) as "i_received!"
        FROM friendships f
        JOIN agents pa   ON pa.id   = f.proposer_agent_id
        JOIN accounts pacc ON pacc.id = pa.account_id
        JOIN agents ta   ON ta.id   = f.target_agent_id
        JOIN accounts tacc ON tacc.id = ta.account_id
        WHERE f.id = $1
        "#,
        id,
        user_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !r.i_proposed && !r.i_received {
        // Friendships are visible to both sides only.
        return Err(ApiError::NotFound);
    }

    Ok(FriendshipDto {
        id: r.id,
        status: r.status,
        proposer: AgentSummary {
            id: r.p_agent_id,
            slug: r.p_agent_slug,
            display_name: r.p_agent_display_name,
            account_id: r.p_acct_id,
            account_slug: r.p_acct_slug,
            account_display_name: r.p_acct_display_name,
        },
        target: AgentSummary {
            id: r.t_agent_id,
            slug: r.t_agent_slug,
            display_name: r.t_agent_display_name,
            account_id: r.t_acct_id,
            account_slug: r.t_acct_slug,
            account_display_name: r.t_acct_display_name,
        },
        proposer_message: r.proposer_message,
        response_message: r.response_message,
        counter_of_id: r.counter_of_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
        decided_at: r.decided_at,
        i_proposed: r.i_proposed,
        i_received: r.i_received,
    })
}

async fn agent_account(db: &PgPool, agent_id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!(r#"SELECT account_id FROM agents WHERE id = $1"#, agent_id)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::NotFound)
}

// ─── GET /v1/friendships ─────────────────────────────────
pub async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<FriendshipDto>>> {
    let direction = q.direction.as_deref().unwrap_or("all");
    if !matches!(direction, "all" | "outbound" | "inbound") {
        return Err(ApiError::InvalidRequest(
            "direction must be all|outbound|inbound".into(),
        ));
    }
    if let Some(s) = q.status.as_deref() {
        if !matches!(s, "proposed" | "accepted" | "rejected" | "cancelled" | "countered") {
            return Err(ApiError::InvalidRequest("invalid status".into()));
        }
    }

    let want_outbound = matches!(direction, "all" | "outbound");
    let want_inbound = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT
            f.id, f.status, f.proposer_message, f.response_message,
            f.counter_of_id, f.created_at, f.updated_at, f.decided_at,
            pa.id   as p_agent_id,   pa.slug as p_agent_slug,   pa.display_name as p_agent_display_name,
            pacc.id as p_acct_id,    pacc.slug as p_acct_slug,  pacc.display_name as p_acct_display_name,
            ta.id   as t_agent_id,   ta.slug as t_agent_slug,   ta.display_name as t_agent_display_name,
            tacc.id as t_acct_id,    tacc.slug as t_acct_slug,  tacc.display_name as t_acct_display_name,
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = pa.account_id AND m.user_id = $1) as "i_proposed!",
            EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ta.account_id AND m.user_id = $1) as "i_received!"
        FROM friendships f
        JOIN agents pa   ON pa.id   = f.proposer_agent_id
        JOIN accounts pacc ON pacc.id = pa.account_id
        JOIN agents ta   ON ta.id   = f.target_agent_id
        JOIN accounts tacc ON tacc.id = ta.account_id
        WHERE
            (
                ($2::boolean AND EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = pa.account_id AND m.user_id = $1))
             OR ($3::boolean AND EXISTS(SELECT 1 FROM account_memberships m WHERE m.account_id = ta.account_id AND m.user_id = $1))
            )
            AND ($4::text IS NULL OR f.status = $4)
        ORDER BY f.updated_at DESC
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
            .map(|r| FriendshipDto {
                id: r.id,
                status: r.status,
                proposer: AgentSummary {
                    id: r.p_agent_id,
                    slug: r.p_agent_slug,
                    display_name: r.p_agent_display_name,
                    account_id: r.p_acct_id,
                    account_slug: r.p_acct_slug,
                    account_display_name: r.p_acct_display_name,
                },
                target: AgentSummary {
                    id: r.t_agent_id,
                    slug: r.t_agent_slug,
                    display_name: r.t_agent_display_name,
                    account_id: r.t_acct_id,
                    account_slug: r.t_acct_slug,
                    account_display_name: r.t_acct_display_name,
                },
                proposer_message: r.proposer_message,
                response_message: r.response_message,
                counter_of_id: r.counter_of_id,
                created_at: r.created_at,
                updated_at: r.updated_at,
                decided_at: r.decided_at,
                i_proposed: r.i_proposed,
                i_received: r.i_received,
            })
            .collect(),
    ))
}

// ─── GET /v1/friendships/{id} ────────────────────────────
pub async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<FriendshipDto>> {
    Ok(Json(fetch_friendship(&state.db, user.user_id, id).await?))
}

// ─── POST /v1/friendships ────────────────────────────────
pub async fn propose(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<ProposeRequest>,
) -> ApiResult<Json<FriendshipDto>> {
    if req.proposer_agent_id == req.target_agent_id {
        return Err(ApiError::InvalidRequest(
            "an agent can't be friends with itself".into(),
        ));
    }

    // Caller must be a member of the proposer agent's account.
    let proposer_account = agent_account(&state.db, req.proposer_agent_id).await?;
    if !user_is_member(&state.db, user.user_id, proposer_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Target must exist; the unique partial index will block duplicate
    // active proposals via a 23505 SQLSTATE.
    let _target_account = agent_account(&state.db, req.target_agent_id).await?;

    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status, proposer_message)
        VALUES ($1, $2, $3, 'proposed', $4)
        RETURNING id
        "#,
        id,
        req.proposer_agent_id,
        req.target_agent_id,
        req.proposer_message,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::Conflict(
                "a proposal between these agents is already in flight".into(),
            )
        }
        other => other.into(),
    })?
    .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("insert returned no row")))?;

    Ok(Json(fetch_friendship(&state.db, user.user_id, inserted.id).await?))
}

// ─── POST /v1/friendships/{id}/cancel ────────────────────
pub async fn cancel(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<FriendshipDto>> {
    let row = sqlx::query!(
        r#"SELECT proposer_agent_id, status FROM friendships WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "proposed" {
        return Err(ApiError::Conflict(format!(
            "friendship is {}; only proposed can be cancelled",
            row.status
        )));
    }

    let proposer_account = agent_account(&state.db, row.proposer_agent_id).await?;
    if !user_is_member(&state.db, user.user_id, proposer_account).await? {
        return Err(ApiError::Forbidden);
    }

    sqlx::query!(
        r#"
        UPDATE friendships
        SET status = 'cancelled', decided_at = now()
        WHERE id = $1 AND status = 'proposed'
        "#,
        id
    )
    .execute(&state.db)
    .await?;

    Ok(Json(fetch_friendship(&state.db, user.user_id, id).await?))
}

// ─── POST /v1/friendships/{id}/accept ────────────────────
pub async fn accept(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ResponseRequest>,
) -> ApiResult<Json<FriendshipDto>> {
    let row = sqlx::query!(
        r#"SELECT target_agent_id, status FROM friendships WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "proposed" {
        return Err(ApiError::Conflict(format!(
            "friendship is {}; can only accept proposed",
            row.status
        )));
    }

    let target_account = agent_account(&state.db, row.target_agent_id).await?;
    if !user_is_member(&state.db, user.user_id, target_account).await? {
        return Err(ApiError::Forbidden);
    }

    sqlx::query!(
        r#"
        UPDATE friendships
        SET status = 'accepted',
            response_message = COALESCE($2, response_message),
            decided_at = now()
        WHERE id = $1 AND status = 'proposed'
        "#,
        id,
        req.response_message,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(fetch_friendship(&state.db, user.user_id, id).await?))
}

// ─── POST /v1/friendships/{id}/reject ────────────────────
pub async fn reject(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ResponseRequest>,
) -> ApiResult<Json<FriendshipDto>> {
    let row = sqlx::query!(
        r#"SELECT target_agent_id, status FROM friendships WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "proposed" {
        return Err(ApiError::Conflict(format!(
            "friendship is {}; can only reject proposed",
            row.status
        )));
    }

    let target_account = agent_account(&state.db, row.target_agent_id).await?;
    if !user_is_member(&state.db, user.user_id, target_account).await? {
        return Err(ApiError::Forbidden);
    }

    sqlx::query!(
        r#"
        UPDATE friendships
        SET status = 'rejected',
            response_message = COALESCE($2, response_message),
            decided_at = now()
        WHERE id = $1 AND status = 'proposed'
        "#,
        id,
        req.response_message,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(fetch_friendship(&state.db, user.user_id, id).await?))
}

// ─── POST /v1/friendships/{id}/counter ───────────────────
pub async fn counter(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CounterRequest>,
) -> ApiResult<Json<FriendshipDto>> {
    let row = sqlx::query!(
        r#"
        SELECT proposer_agent_id, target_agent_id, status
        FROM friendships WHERE id = $1
        "#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "proposed" {
        return Err(ApiError::Conflict(format!(
            "friendship is {}; only proposed can be countered",
            row.status
        )));
    }

    let target_account = agent_account(&state.db, row.target_agent_id).await?;
    if !user_is_member(&state.db, user.user_id, target_account).await? {
        return Err(ApiError::Forbidden);
    }

    let new_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;

    // Mark the original countered.
    sqlx::query!(
        r#"
        UPDATE friendships
        SET status = 'countered',
            response_message = COALESCE($2, response_message),
            decided_at = now()
        WHERE id = $1 AND status = 'proposed'
        "#,
        id,
        req.response_message,
    )
    .execute(&mut *tx)
    .await?;

    // Open the reverse proposal.
    sqlx::query!(
        r#"
        INSERT INTO friendships
            (id, proposer_agent_id, target_agent_id, status, proposer_message, counter_of_id)
        VALUES ($1, $2, $3, 'proposed', $4, $5)
        "#,
        new_id,
        row.target_agent_id,    // counter swaps direction
        row.proposer_agent_id,
        req.proposer_message,
        id,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::Conflict(
                "the reverse direction already has an active proposal".into(),
            )
        }
        other => other.into(),
    })?;

    tx.commit().await?;

    Ok(Json(fetch_friendship(&state.db, user.user_id, new_id).await?))
}
