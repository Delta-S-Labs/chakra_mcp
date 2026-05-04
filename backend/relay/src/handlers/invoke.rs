//! Async, inbox-pull invocations — the v0.1.0 SDK contract surface.
//!
//! These endpoints predate the A2A migration and are preserved
//! unchanged on `relay.chakramcp.com` per discovery spec Override #2
//! (the "two-host surface model"). The published `chakramcp` Python
//! SDK + `@chakramcp/sdk` TypeScript package at v0.1.0 use exactly
//! these URLs.
//!
//! POST /v1/invoke
//!   Grantee asks the relay to deliver `input` to the granter for
//!   capability `C`. The relay validates the grant, enqueues a row in
//!   `pending`, and returns the invocation id immediately. No outbound
//!   HTTP — the granter side will pull it from their inbox.
//!
//! GET /v1/inbox?agent_id=…&limit=…
//!   The granter's owner pulls oldest pending invocations targeting the
//!   named agent and atomically marks them `in_progress` (so two
//!   concurrent pollers don't claim the same row). They then run the
//!   work locally and report the result.
//!
//! POST /v1/invocations/{id}/result
//!   The granter side reports succeeded or failed with output / error.
//!   Only the user who claimed the row (or another member of the
//!   granter's account) can post a result, and only while it's
//!   in_progress.
//!
//! GET /v1/invocations / GET /v1/invocations/{id}
//!   Audit + status polling. The grantee polls /{id} until status is
//!   terminal (succeeded | failed | timeout | rejected), then reads
//!   output_preview / error_message off the row.
//!
//! ── Relationship to the A2A migration (D6) ─────────────────────
//!
//! These endpoints share the `relay_invocations` table with the new
//! A2A surfaces (D5):
//!
//! - `inbox_bridge::park` (D5c) inserts the same `status='pending'`
//!   shape this `/v1/inbox` polls. A v0.1.0 granter SDK transparently
//!   picks up calls originating from generic A2A clients hitting
//!   `/agents/<acct>/<slug>/a2a/jsonrpc` — no SDK upgrade needed.
//! - `inbox_bridge::get_task` (D5d) reads the same row from the
//!   caller's side and shapes it as an A2A Task. The same row is
//!   both an "Invocation" (legacy poll) and a "Task" (A2A poll).
//! - `forwarder::forward_push` (D5b) writes a terminal-status row
//!   directly. v0.1.0 SDKs invoking via `/v1/invoke` against push
//!   targets currently see a pending row that no one delivers —
//!   push agents are reached only via the new A2A endpoint in v1.
//!   The scheduler-demo (and every v0.1.0 SDK use case to date)
//!   targets pull-mode agents and is unaffected.
//!
//! Regression coverage: `legacy_v01_contract_tests` at the bottom of
//! this module exercises the full v0.1.0 lifecycle on the post-D5
//! schema as the D6 acceptance gate.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

const PREVIEW_BYTE_LIMIT: usize = 16 * 1024;
const INBOX_DEFAULT_LIMIT: i64 = 25;
const INBOX_MAX_LIMIT: i64 = 100;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InvokeRequest {
    pub grant_id: Uuid,
    /// The agent the caller is invoking AS — must be a member of its
    /// account, must match the grant's grantee_agent_id.
    pub grantee_agent_id: Uuid,
    pub input: Value,
}

#[derive(Debug, Serialize)]
pub struct InvokeResponse {
    pub invocation_id: Uuid,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InvocationDto {
    pub id: Uuid,
    pub grant_id: Option<Uuid>,
    pub granter_agent_id: Option<Uuid>,
    pub granter_display_name: Option<String>,
    pub grantee_agent_id: Option<Uuid>,
    pub grantee_display_name: Option<String>,
    pub capability_id: Option<Uuid>,
    pub capability_name: String,
    pub status: String,
    pub elapsed_ms: i32,
    pub error_message: Option<String>,
    pub input_preview: Option<Value>,
    pub output_preview: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    /// True when the requesting user is on the granter side.
    pub i_served: bool,
    /// True when the requesting user is on the grantee side.
    pub i_invoked: bool,
    /// Trust context bundled in by the relay on inbox.pull responses
    /// only — these fields are populated when the relay just verified
    /// friendship + grant before delivering this row, so the receiving
    /// agent doesn't need to re-query. Always None on list/get
    /// (audit-log) endpoints, where the friendship/grant state at
    /// query time may differ from when the invocation was authorised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendship_context: Option<FriendshipContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_context: Option<GrantContext>,
}

/// Friendship details the relay verified before queuing this
/// invocation. Trust the assertions here without re-querying.
#[derive(Debug, Serialize)]
pub struct FriendshipContext {
    pub id: Uuid,
    pub status: String,
    pub proposer_agent_id: Uuid,
    pub target_agent_id: Uuid,
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// Grant details the relay verified before queuing this invocation.
#[derive(Debug, Serialize)]
pub struct GrantContext {
    pub id: Uuid,
    pub status: String,
    pub granter_agent_id: Uuid,
    pub grantee_agent_id: Uuid,
    pub capability_id: Uuid,
    pub capability_name: String,
    pub capability_visibility: String,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// "outbound" (I served) | "inbound" (I invoked) | omitted for both.
    pub direction: Option<String>,
    pub agent_id: Option<Uuid>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InboxQuery {
    pub agent_id: Uuid,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ResultRequest {
    /// "succeeded" or "failed".
    pub status: String,
    pub output: Option<Value>,
    pub error: Option<String>,
}

// ─── Helpers ─────────────────────────────────────────────

fn truncate_for_audit(value: &Value) -> Value {
    let s = value.to_string();
    if s.len() <= PREVIEW_BYTE_LIMIT {
        value.clone()
    } else {
        serde_json::json!({
            "__chakramcp_truncated__": true,
            "original_byte_length": s.len(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_terminal(
    db: &PgPool,
    grant_id: Option<Uuid>,
    granter_agent_id: Option<Uuid>,
    grantee_agent_id: Option<Uuid>,
    capability_id: Option<Uuid>,
    capability_name: &str,
    invoked_by_user_id: Uuid,
    status: &str,
    elapsed_ms: i32,
    error_message: Option<&str>,
    input_preview: Option<&Value>,
) -> Result<Uuid, ApiError> {
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, elapsed_ms,
             error_message, input_preview)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        id,
        grant_id,
        granter_agent_id,
        grantee_agent_id,
        capability_id,
        capability_name,
        invoked_by_user_id,
        status,
        elapsed_ms,
        error_message,
        input_preview.cloned().unwrap_or(Value::Null),
    )
    .execute(db)
    .await?;
    Ok(id)
}

// ─── POST /v1/invoke (enqueue) ───────────────────────────
pub async fn invoke(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<InvokeRequest>,
) -> Result<(StatusCode, Json<InvokeResponse>), ApiError> {
    let input_preview = truncate_for_audit(&req.input);

    // Resolve the grant + agents + capability.
    let row = sqlx::query!(
        r#"
        SELECT
            g.id as grant_id, g.status as grant_status,
            g.granter_agent_id, g.grantee_agent_id, g.capability_id,
            g.expires_at,
            ga.account_id as granter_account_id,
            ea.account_id as grantee_account_id,
            cap.name as capability_name
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        WHERE g.id = $1
        "#,
        req.grant_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.grantee_agent_id != req.grantee_agent_id {
        let id = record_terminal(
            &state.db, Some(row.grant_id), Some(row.granter_agent_id),
            Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
            user.user_id, "rejected", 0,
            Some("grantee_agent_id does not match the grant"),
            Some(&input_preview),
        ).await?;
        return Ok((StatusCode::BAD_REQUEST, Json(InvokeResponse {
            invocation_id: id, status: "rejected".into(),
            error: Some("grantee_agent_id does not match the grant".into()),
        })));
    }

    // Caller must be a member of the grantee's account.
    if !user_is_member(&state.db, user.user_id, row.grantee_account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Grant must be active and not expired.
    if row.grant_status != "active" {
        let msg = format!("grant is {}; only active grants can be invoked", row.grant_status);
        let id = record_terminal(
            &state.db, Some(row.grant_id), Some(row.granter_agent_id),
            Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
            user.user_id, "rejected", 0, Some(&msg), Some(&input_preview),
        ).await?;
        return Ok((StatusCode::CONFLICT, Json(InvokeResponse {
            invocation_id: id, status: "rejected".into(), error: Some(msg),
        })));
    }
    if let Some(exp) = row.expires_at {
        if exp <= Utc::now() {
            let msg = "grant has expired".to_string();
            let id = record_terminal(
                &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                user.user_id, "rejected", 0, Some(&msg), Some(&input_preview),
            ).await?;
            return Ok((StatusCode::CONFLICT, Json(InvokeResponse {
                invocation_id: id, status: "rejected".into(), error: Some(msg),
            })));
        }
    }

    // Enqueue the invocation. Granter side will pull it from /v1/inbox.
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, input_preview)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
        "#,
        id,
        row.grant_id,
        row.granter_agent_id,
        row.grantee_agent_id,
        row.capability_id,
        row.capability_name,
        user.user_id,
        input_preview,
    )
    .execute(&state.db)
    .await?;

    Ok((StatusCode::ACCEPTED, Json(InvokeResponse {
        invocation_id: id, status: "pending".into(), error: None,
    })))
}

// ─── GET /v1/inbox?agent_id=X[&limit=N] ──────────────────
pub async fn inbox(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<InboxQuery>,
) -> ApiResult<Json<Vec<InvocationDto>>> {
    let limit = q.limit.unwrap_or(INBOX_DEFAULT_LIMIT).clamp(1, INBOX_MAX_LIMIT);

    // Caller must be a member of the agent's account.
    let agent_account = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        q.agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !user_is_member(&state.db, user.user_id, agent_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Atomically claim the oldest pending rows for this agent.
    // FOR UPDATE SKIP LOCKED lets concurrent pollers safely pull
    // disjoint batches without blocking each other.
    let mut tx = state.db.begin().await?;
    let claimed = sqlx::query!(
        r#"
        WITH picked AS (
            SELECT id FROM relay_invocations
            WHERE granter_agent_id = $1
              AND status = 'pending'
            ORDER BY created_at ASC
            LIMIT $2
            FOR UPDATE SKIP LOCKED
        )
        UPDATE relay_invocations i
        SET status = 'in_progress',
            claimed_at = now(),
            claimed_by_user_id = $3
        FROM picked
        WHERE i.id = picked.id
        RETURNING i.id
        "#,
        q.agent_id,
        limit,
        user.user_id,
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    if claimed.is_empty() {
        return Ok(Json(vec![]));
    }

    let ids: Vec<Uuid> = claimed.into_iter().map(|r| r.id).collect();
    // Bundle in friendship + grant context. The relay just verified
    // both before queuing this row, so the receiving agent doesn't
    // need to re-check — saves an LLM tool call (or three) per
    // invocation.
    let rows = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at, i.claimed_at,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",

            g.status              as "g_status?",
            g.granter_agent_id    as "g_granter_agent_id?",
            g.grantee_agent_id    as "g_grantee_agent_id?",
            g.capability_id       as "g_capability_id?",
            cap.visibility        as "g_capability_visibility?",
            g.granted_at          as "g_granted_at?",
            g.expires_at          as "g_expires_at?",

            f.id                  as "f_id?",
            f.status              as "f_status?",
            f.proposer_agent_id   as "f_proposer_agent_id?",
            f.target_agent_id     as "f_target_agent_id?",
            f.proposer_message    as "f_proposer_message?",
            f.response_message    as "f_response_message?",
            f.decided_at          as "f_decided_at?"
        FROM relay_invocations i
        LEFT JOIN agents ga             ON ga.id  = i.granter_agent_id
        LEFT JOIN agents ea             ON ea.id  = i.grantee_agent_id
        LEFT JOIN grants g              ON g.id   = i.grant_id
        LEFT JOIN agent_capabilities cap ON cap.id = g.capability_id
        LEFT JOIN LATERAL (
            SELECT *
            FROM friendships f2
            WHERE f2.status = 'accepted'
              AND (
                  (f2.proposer_agent_id = i.granter_agent_id AND f2.target_agent_id = i.grantee_agent_id)
               OR (f2.proposer_agent_id = i.grantee_agent_id AND f2.target_agent_id = i.granter_agent_id)
              )
            ORDER BY f2.decided_at DESC
            LIMIT 1
        ) f ON true
        WHERE i.id = ANY($1)
        ORDER BY i.created_at ASC
        "#,
        &ids,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let grant_context = match (
                    r.grant_id,
                    r.g_status.clone(),
                    r.g_granter_agent_id,
                    r.g_grantee_agent_id,
                    r.g_capability_id,
                    r.g_capability_visibility.clone(),
                    r.g_granted_at,
                ) {
                    (
                        Some(grant_id),
                        Some(status),
                        Some(granter),
                        Some(grantee),
                        Some(capability_id),
                        Some(visibility),
                        Some(granted_at),
                    ) => Some(GrantContext {
                        id: grant_id,
                        status,
                        granter_agent_id: granter,
                        grantee_agent_id: grantee,
                        capability_id,
                        capability_name: r.capability_name.clone(),
                        capability_visibility: visibility,
                        granted_at,
                        expires_at: r.g_expires_at,
                    }),
                    _ => None,
                };
                let friendship_context = match (
                    r.f_id,
                    r.f_status.clone(),
                    r.f_proposer_agent_id,
                    r.f_target_agent_id,
                ) {
                    (Some(id), Some(status), Some(proposer), Some(target)) => {
                        Some(FriendshipContext {
                            id,
                            status,
                            proposer_agent_id: proposer,
                            target_agent_id: target,
                            proposer_message: r.f_proposer_message,
                            response_message: r.f_response_message,
                            decided_at: r.f_decided_at,
                        })
                    }
                    _ => None,
                };

                InvocationDto {
                    id: r.id,
                    grant_id: r.grant_id,
                    granter_agent_id: r.granter_agent_id,
                    granter_display_name: r.granter_display_name,
                    grantee_agent_id: r.grantee_agent_id,
                    grantee_display_name: r.grantee_display_name,
                    capability_id: r.capability_id,
                    capability_name: r.capability_name,
                    status: r.status,
                    elapsed_ms: r.elapsed_ms,
                    error_message: r.error_message,
                    input_preview: r.input_preview,
                    output_preview: r.output_preview,
                    created_at: r.created_at,
                    claimed_at: r.claimed_at,
                    i_served: true,
                    i_invoked: false,
                    friendship_context,
                    grant_context,
                }
            })
            .collect(),
    ))
}

// ─── POST /v1/invocations/{id}/result ────────────────────
pub async fn report_result(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ResultRequest>,
) -> ApiResult<Json<InvocationDto>> {
    if !matches!(req.status.as_str(), "succeeded" | "failed") {
        return Err(ApiError::InvalidRequest(
            "status must be 'succeeded' or 'failed'".into(),
        ));
    }

    let row = sqlx::query!(
        r#"
        SELECT i.status, i.created_at, i.claimed_at,
               ga.account_id as "granter_account_id?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        WHERE i.id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "in_progress" {
        return Err(ApiError::Conflict(format!(
            "invocation is {}; only in_progress can be completed",
            row.status
        )));
    }

    // Auth: caller must be a member of the granter's account. The agent
    // could be deleted in the meantime; if so, granter_account_id is
    // null and no one can report a result — surface as forbidden.
    let granter_account = row.granter_account_id.ok_or(ApiError::Forbidden)?;
    if !user_is_member(&state.db, user.user_id, granter_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Wall time from enqueue to result.
    let elapsed_ms = (Utc::now() - row.created_at).num_milliseconds().clamp(0, i32::MAX as i64) as i32;
    let output_preview = req.output.as_ref().map(truncate_for_audit);

    sqlx::query!(
        r#"
        UPDATE relay_invocations
        SET status = $2,
            elapsed_ms = $3,
            error_message = $4,
            output_preview = $5
        WHERE id = $1 AND status = 'in_progress'
        "#,
        id,
        req.status,
        elapsed_ms,
        req.error.as_deref(),
        output_preview.unwrap_or(Value::Null),
    )
    .execute(&state.db)
    .await?;

    let r = fetch_one(&state.db, user.user_id, id).await?;
    Ok(Json(r))
}

// ─── GET /v1/invocations ─────────────────────────────────
pub async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<InvocationDto>>> {
    let direction = q.direction.as_deref().unwrap_or("all");
    if !matches!(direction, "all" | "outbound" | "inbound") {
        return Err(ApiError::InvalidRequest(
            "direction must be all|outbound|inbound".into(),
        ));
    }
    if let Some(s) = q.status.as_deref() {
        if !matches!(s, "pending" | "in_progress" | "rejected" | "succeeded" | "failed" | "timeout") {
            return Err(ApiError::InvalidRequest("invalid status".into()));
        }
    }

    let want_outbound = matches!(direction, "all" | "outbound");
    let want_inbound = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at, i.claimed_at,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ga.account_id
            ) as "i_served?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ea.account_id
            ) as "i_invoked?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE
            (
                ($2::boolean AND ga.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                ))
             OR ($3::boolean AND ea.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                ))
            )
            AND ($4::uuid IS NULL OR i.granter_agent_id = $4 OR i.grantee_agent_id = $4)
            AND ($5::text IS NULL OR i.status = $5)
        ORDER BY i.created_at DESC
        LIMIT 200
        "#,
        user.user_id,
        want_outbound,
        want_inbound,
        q.agent_id,
        q.status,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| InvocationDto {
                id: r.id,
                grant_id: r.grant_id,
                granter_agent_id: r.granter_agent_id,
                granter_display_name: r.granter_display_name,
                grantee_agent_id: r.grantee_agent_id,
                grantee_display_name: r.grantee_display_name,
                capability_id: r.capability_id,
                capability_name: r.capability_name,
                status: r.status,
                elapsed_ms: r.elapsed_ms,
                error_message: r.error_message,
                input_preview: r.input_preview,
                output_preview: r.output_preview,
                created_at: r.created_at,
                claimed_at: r.claimed_at,
                i_served: r.i_served.unwrap_or(false),
                i_invoked: r.i_invoked.unwrap_or(false),
                friendship_context: None,
                grant_context: None,
            })
            .collect(),
    ))
}

// ─── GET /v1/invocations/{id} ────────────────────────────
pub async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InvocationDto>> {
    Ok(Json(fetch_one(&state.db, user.user_id, id).await?))
}

async fn fetch_one(db: &PgPool, user_id: Uuid, id: Uuid) -> Result<InvocationDto, ApiError> {
    let r = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at, i.claimed_at,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ga.account_id
            ) as "i_served?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ea.account_id
            ) as "i_invoked?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.id = $2
        "#,
        user_id,
        id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let i_served = r.i_served.unwrap_or(false);
    let i_invoked = r.i_invoked.unwrap_or(false);
    if !i_served && !i_invoked {
        return Err(ApiError::NotFound);
    }

    Ok(InvocationDto {
        id: r.id,
        grant_id: r.grant_id,
        granter_agent_id: r.granter_agent_id,
        granter_display_name: r.granter_display_name,
        grantee_agent_id: r.grantee_agent_id,
        grantee_display_name: r.grantee_display_name,
        capability_id: r.capability_id,
        capability_name: r.capability_name,
        status: r.status,
        elapsed_ms: r.elapsed_ms,
        error_message: r.error_message,
        input_preview: r.input_preview,
        output_preview: r.output_preview,
        created_at: r.created_at,
        claimed_at: r.claimed_at,
        i_served,
        i_invoked,
        friendship_context: None,
        grant_context: None,
    })
}

#[cfg(test)]
mod legacy_v01_contract_tests {
    //! D6 regression gate: the v0.1.0 SDK contract for the legacy
    //! pull-mode endpoints must continue to work post-A2A migration,
    //! since v0.1.0 of `chakramcp` (Python) and `@chakramcp/sdk` (TS)
    //! are already published and depend on these exact endpoint
    //! shapes:
    //!
    //!   POST /v1/invoke                 — caller submits work
    //!   GET  /v1/invocations/{id}       — caller polls for terminal
    //!   GET  /v1/inbox?agent_id=…       — granter polls for parked
    //!   POST /v1/invocations/{id}/result — granter reports outcome
    //!
    //! The scheduler-demo (`examples/scheduler-demo/`) is a
    //! published-on-the-website v0.1.0 SDK demo that uses exactly
    //! this flow. If this test fails, the demo fails. D5 doesn't
    //! touch any of these handlers — but the schema migrations
    //! from D1 *did* touch the agents/accounts tables, so we
    //! verify the contract holds end-to-end.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sha2::{Digest, Sha256};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            // Off — legacy endpoints don't depend on it. Keeping it
            // false also proves the v0.1.0 contract works for
            // self-hosters who haven't enabled the new A2A surfaces.
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    /// Seed the same shape as `examples/scheduler-demo/setup.py`:
    /// two accounts, one agent each, friendship, grant. Returns the
    /// caller's ck_ token, both agent ids, and the grant id.
    struct DemoFixture {
        caller_token: String,
        caller_user_id: Uuid,
        granter_agent_id: Uuid,
        grantee_agent_id: Uuid,
        grant_id: Uuid,
        capability_id: Uuid,
    }

    async fn seed_demo(pool: &PgPool) -> DemoFixture {
        let alice_user = Uuid::now_v7(); // granter / inbox.serve side
        let bob_user = Uuid::now_v7(); // grantee / invoke side
        for (uid, name) in [(alice_user, "Alice"), (bob_user, "Bob")] {
            sqlx::query!(
                r#"INSERT INTO users (id, email, display_name, password_hash)
                   VALUES ($1, $2, $3, 'x')"#,
                uid,
                format!("{name}-{uid}@t.local"),
                name,
            )
            .execute(pool)
            .await
            .unwrap();
        }

        // Bob (the caller) gets an API key — same shape v0.1.0 SDK uses.
        let bob_ck = format!("ck_demo_{bob_user}");
        let mut h = Sha256::new();
        h.update(bob_ck.as_bytes());
        let kh = hex::encode(h.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'demo', 'ck_demo')"#,
            Uuid::now_v7(),
            bob_user,
            kh,
        )
        .execute(pool)
        .await
        .unwrap();

        let alice_account = Uuid::now_v7();
        let bob_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Alice', 'individual', $3),
                      ($4, $5, 'Bob',   'individual', $6)"#,
            alice_account,
            format!("alice-{alice_account}"),
            alice_user,
            bob_account,
            format!("bob-{bob_account}"),
            bob_user,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner'),
                      ($4, $5, $6, 'owner')"#,
            Uuid::now_v7(),
            alice_account,
            alice_user,
            Uuid::now_v7(),
            bob_account,
            bob_user,
        )
        .execute(pool)
        .await
        .unwrap();

        // Two agents: alice-scheduler (granter) + bob-caller (grantee).
        let alice_agent = Uuid::now_v7();
        let bob_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'alice-scheduler', 'Alice Scheduler', 'network'),
                      ($3, $4, 'bob-caller',      'Bob Caller',      'network')"#,
            alice_agent,
            alice_account,
            bob_agent,
            bob_account,
        )
        .execute(pool)
        .await
        .unwrap();

        // The propose_slots capability on alice-scheduler.
        let cap = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, 'propose_slots', 'Propose meeting slots.',
                       '{"type":"object"}'::jsonb,
                       '{"type":"object"}'::jsonb,
                       'network')"#,
            cap,
            alice_agent,
        )
        .execute(pool)
        .await
        .unwrap();

        // Friendship + Grant.
        sqlx::query!(
            r#"INSERT INTO friendships
                  (id, proposer_agent_id, target_agent_id, status, decided_at)
               VALUES ($1, $2, $3, 'accepted', now())"#,
            Uuid::now_v7(),
            alice_agent,
            bob_agent,
        )
        .execute(pool)
        .await
        .unwrap();
        let grant = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO grants
                  (id, granter_agent_id, grantee_agent_id, capability_id, status)
               VALUES ($1, $2, $3, $4, 'active')"#,
            grant,
            alice_agent,
            bob_agent,
            cap,
        )
        .execute(pool)
        .await
        .unwrap();

        DemoFixture {
            caller_token: bob_ck,
            caller_user_id: bob_user,
            granter_agent_id: alice_agent,
            grantee_agent_id: bob_agent,
            grant_id: grant,
            capability_id: cap,
        }
    }

    /// Full flow mirrors what `chakramcp.AsyncChakraMCP.invoke_and_wait`
    /// + `chakramcp.AsyncChakraMCP.inbox.serve` do at the wire level.
    /// If this test passes, the scheduler-demo passes.
    #[sqlx::test(migrations = "../migrations")]
    async fn v01_pull_mode_full_lifecycle(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        // ── Bob: POST /v1/invoke ────────────────────────────
        let invoke_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/invoke")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "grant_id": f.grant_id,
                            "grantee_agent_id": f.grantee_agent_id,
                            "input": {"duration_min": 30, "within_days": 7}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invoke_res.status(), StatusCode::ACCEPTED);
        let invoke_body: serde_json::Value =
            serde_json::from_slice(&invoke_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(invoke_body["status"], "pending");
        let invocation_id: Uuid = invoke_body["invocation_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap();

        // ── Bob: GET /v1/invocations/{id} (still pending) ───
        let poll_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/invocations/{invocation_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(poll_res.status(), StatusCode::OK);
        let poll_body: serde_json::Value = serde_json::from_slice(
            &poll_res.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        assert_eq!(poll_body["status"], "pending");

        // ── Alice: GET /v1/inbox?agent_id=<alice-scheduler> ─
        // Need a token for Alice. Mint one quickly.
        let alice_user_id = sqlx::query_scalar!(
            "SELECT owner_user_id FROM accounts a JOIN agents g ON g.account_id=a.id WHERE g.id=$1",
            f.granter_agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
        let alice_ck = format!("ck_demo_alice_{alice_user_id}");
        let mut h = Sha256::new();
        h.update(alice_ck.as_bytes());
        let kh = hex::encode(h.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'demo', 'ck_demo')"#,
            Uuid::now_v7(),
            alice_user_id,
            kh,
        )
        .execute(&pool)
        .await
        .unwrap();

        let inbox_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/inbox?agent_id={}", f.granter_agent_id))
                    .header(header::AUTHORIZATION, format!("Bearer {alice_ck}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(inbox_res.status(), StatusCode::OK);
        let inbox_body: serde_json::Value = serde_json::from_slice(
            &inbox_res.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        // The legacy contract returns an array of invocations.
        let items = inbox_body
            .get("invocations")
            .or_else(|| inbox_body.as_array().map(|_| &inbox_body))
            .and_then(|v| v.as_array())
            .expect("inbox response should contain an invocations array");
        assert!(
            items.iter().any(|inv| inv["id"] == invocation_id.to_string()),
            "Alice's inbox should include the parked invocation"
        );

        // ── Alice: POST /v1/invocations/{id}/result ─────────
        let result_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/invocations/{invocation_id}/result"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_ck}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "status": "succeeded",
                            "output": { "slots": ["2026-05-04T10:00:00Z"] }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            result_res.status().is_success(),
            "result post should succeed; got {}",
            result_res.status()
        );

        // ── Bob: GET /v1/invocations/{id} (now succeeded) ───
        let poll2 = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/invocations/{invocation_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let poll2_body: serde_json::Value = serde_json::from_slice(
            &poll2.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
        assert_eq!(poll2_body["status"], "succeeded");
        assert_eq!(
            poll2_body["output_preview"]["slots"][0],
            "2026-05-04T10:00:00Z"
        );

        // Defensive use of unused values.
        let _ = (
            f.caller_user_id,
            f.capability_id,
            f.grantee_agent_id,
            invocation_id,
        );
    }
}
