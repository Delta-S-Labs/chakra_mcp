//! Owner consent decisions (spec §4.4, §7, §8).
//!
//! GET  /v1/approvals?state=waiting|active|history
//! GET  /v1/approvals/{id}
//! POST /v1/approvals/{id}/approve   {valid_for?, max_requests?}
//! POST /v1/approvals/{id}/deny      {note?}
//! POST /v1/consents/{id}/revoke
//!   A person signed in to the web app only (`InteractiveUser`): never an
//!   API key or a token minted for an agent, so an agent can't approve its
//!   own requests. Deciding needs the approver role (consent::approvers).
//!
//! GET  /v1/consent-requests/{id}
//!   Any credential of either side: the request's status and terms, never
//!   the held input.
//!
//! Viewing and deciding are `chakramcp_shared::consent::decide`, which the
//! app's built-in approval page (self-hosted servers) uses too; this module
//! is the HTTP layer and the lists.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use chakramcp_shared::consent::decide::{self, AgentRef};
use chakramcp_shared::consent::terms::Validity;

use crate::auth::{AuthUser, InteractiveUser};
use crate::state::RelayState;

/// How far back the history list goes, and how long a list may be.
const HISTORY_DAYS: i32 = 90;
const LIST_LIMIT: i64 = 200;

// ─── DTOs ────────────────────────────────────────────────

/// A row in the waiting and history lists.
#[derive(Debug, Serialize)]
pub struct ApprovalSummary {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub invocation_id: Uuid,
    pub capability_name: Option<String>,
    pub target: AgentRef,
    pub requester_agent_id: Uuid,
    pub requester_display_name: Option<String>,
    pub requester_account_display_name: Option<String>,
    pub terms: Value,
    pub approved_terms: Option<Value>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub decided_by_display_name: Option<String>,
    /// Approved by a standing consent rather than a person.
    pub covered: bool,
}

/// A row in the active consents list.
#[derive(Debug, Serialize)]
pub struct ConsentSummary {
    pub id: Uuid,
    pub capability_id: Uuid,
    pub capability_name: Option<String>,
    pub target: AgentRef,
    pub requester_agent_id: Uuid,
    pub requester_display_name: Option<String>,
    pub requester_account_display_name: Option<String>,
    pub purpose: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub max_requests: Option<i32>,
    pub used: i32,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub state: Option<String>,
}

/// An optional JSON body: empty means all defaults.
fn optional_body<T: for<'de> Deserialize<'de> + Default>(body: &Bytes) -> ApiResult<T> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(body).map_err(|e| ApiError::InvalidRequest(format!("bad body: {e}")))
}

// ─── GET /v1/approvals ───────────────────────────────────

pub async fn list(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    match q.state.as_deref().unwrap_or("waiting") {
        "waiting" => Ok(Json(json!(waiting(&state, user.user_id).await?))),
        "active" => Ok(Json(json!(active(&state, user.user_id).await?))),
        "history" => Ok(Json(json!(history(&state, user.user_id).await?))),
        _ => Err(ApiError::InvalidRequest(
            "state must be waiting|active|history".into(),
        )),
    }
}

/// Pending requests the user may decide. The approver rule is
/// `consent::approvers::may_decide`, inlined for the whole list.
async fn waiting(state: &RelayState, user_id: Uuid) -> ApiResult<Vec<ApprovalSummary>> {
    let rows = sqlx::query!(
        r#"
        SELECT cr.id, cr.kind, cr.status, cr.invocation_id, cr.terms, cr.approved_terms,
               cr.expires_at, cr.created_at, cr.decided_at, cr.decision_note,
               cr.granter_agent_id, ga.display_name AS granter_display_name,
               cr.grantee_agent_id, ea.display_name AS "grantee_display_name?",
               eacc.display_name AS "grantee_account_display_name?",
               cap.name AS "capability_name?"
          FROM consent_requests cr
          JOIN agents ga ON ga.id = cr.granter_agent_id
          JOIN account_memberships m ON m.account_id = ga.account_id AND m.user_id = $1
          LEFT JOIN agents ea ON ea.id = cr.grantee_agent_id
          LEFT JOIN accounts eacc ON eacc.id = cr.grantee_account_id
          LEFT JOIN agent_capabilities cap ON cap.id = cr.capability_id
         WHERE cr.status = 'pending'
           AND cr.expires_at > now()
           AND (m.user_id = ANY(COALESCE(cap.consent_approvers, '{}'))
                OR (m.role IN ('owner', 'admin')
                    AND NOT EXISTS (
                        SELECT 1 FROM account_memberships listed
                         WHERE listed.account_id = ga.account_id
                           AND listed.user_id = ANY(COALESCE(cap.consent_approvers, '{}')))))
         ORDER BY cr.created_at DESC
         LIMIT $2
        "#,
        user_id,
        LIST_LIMIT,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ApprovalSummary {
            id: r.id,
            kind: r.kind,
            status: r.status,
            invocation_id: r.invocation_id,
            capability_name: r.capability_name,
            target: AgentRef {
                id: r.granter_agent_id,
                display_name: Some(r.granter_display_name),
            },
            requester_agent_id: r.grantee_agent_id,
            requester_display_name: r.grantee_display_name,
            requester_account_display_name: r.grantee_account_display_name,
            terms: r.terms,
            approved_terms: r.approved_terms,
            expires_at: r.expires_at,
            created_at: r.created_at,
            decided_at: r.decided_at,
            decision_note: r.decision_note,
            decided_by_display_name: None,
            covered: false,
        })
        .collect())
}

/// Consents that still cover requests (unrevoked, unexpired, not used up)
/// on agents in the user's accounts.
async fn active(state: &RelayState, user_id: Uuid) -> ApiResult<Vec<ConsentSummary>> {
    let rows = sqlx::query!(
        r#"
        SELECT c.id, c.capability_id, cap.name AS "capability_name?",
               c.granter_agent_id, ga.display_name AS granter_display_name,
               c.grantee_agent_id, ea.display_name AS "grantee_display_name?",
               eacc.display_name AS "grantee_account_display_name?",
               c.purpose, c.valid_from, c.valid_until, c.max_requests, c.used
          FROM consents c
          JOIN agents ga ON ga.id = c.granter_agent_id
          JOIN account_memberships m ON m.account_id = ga.account_id AND m.user_id = $1
          LEFT JOIN agents ea ON ea.id = c.grantee_agent_id
          LEFT JOIN accounts eacc ON eacc.id = ea.account_id
          LEFT JOIN agent_capabilities cap ON cap.id = c.capability_id
         WHERE c.revoked_at IS NULL AND c.valid_until > now()
           -- A consent whose requests are used up covers nothing more.
           AND (c.max_requests IS NULL OR c.used < c.max_requests)
         ORDER BY c.created_at DESC
         LIMIT $2
        "#,
        user_id,
        LIST_LIMIT,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ConsentSummary {
            id: r.id,
            capability_id: r.capability_id,
            capability_name: r.capability_name,
            target: AgentRef {
                id: r.granter_agent_id,
                display_name: Some(r.granter_display_name),
            },
            requester_agent_id: r.grantee_agent_id,
            requester_display_name: r.grantee_display_name,
            requester_account_display_name: r.grantee_account_display_name,
            purpose: r.purpose,
            valid_from: r.valid_from,
            valid_until: r.valid_until,
            max_requests: r.max_requests,
            used: r.used,
        })
        .collect())
}

/// Decided requests to agents in the user's accounts, last 90 days.
async fn history(state: &RelayState, user_id: Uuid) -> ApiResult<Vec<ApprovalSummary>> {
    let rows = sqlx::query!(
        r#"
        SELECT cr.id, cr.kind, cr.status, cr.invocation_id, cr.terms, cr.approved_terms,
               cr.expires_at, cr.created_at, cr.decided_at, cr.decision_note,
               cr.consent_id, cr.decided_by_user_id,
               cr.granter_agent_id, ga.display_name AS granter_display_name,
               cr.grantee_agent_id, ea.display_name AS "grantee_display_name?",
               eacc.display_name AS "grantee_account_display_name?",
               cap.name AS "capability_name?",
               u.display_name AS "decided_by_display_name?"
          FROM consent_requests cr
          JOIN agents ga ON ga.id = cr.granter_agent_id
          JOIN account_memberships m ON m.account_id = ga.account_id AND m.user_id = $1
          LEFT JOIN agents ea ON ea.id = cr.grantee_agent_id
          LEFT JOIN accounts eacc ON eacc.id = cr.grantee_account_id
          LEFT JOIN agent_capabilities cap ON cap.id = cr.capability_id
          LEFT JOIN users u ON u.id = cr.decided_by_user_id
         WHERE cr.status <> 'pending'
           AND cr.created_at > now() - make_interval(days => $2)
         ORDER BY cr.created_at DESC
         LIMIT $3
        "#,
        user_id,
        HISTORY_DAYS,
        LIST_LIMIT,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ApprovalSummary {
            id: r.id,
            kind: r.kind,
            covered: r.status == "approved"
                && r.consent_id.is_some()
                && r.decided_by_user_id.is_none(),
            status: r.status,
            invocation_id: r.invocation_id,
            capability_name: r.capability_name,
            target: AgentRef {
                id: r.granter_agent_id,
                display_name: Some(r.granter_display_name),
            },
            requester_agent_id: r.grantee_agent_id,
            requester_display_name: r.grantee_display_name,
            requester_account_display_name: r.grantee_account_display_name,
            terms: r.terms,
            approved_terms: r.approved_terms,
            expires_at: r.expires_at,
            created_at: r.created_at,
            decided_at: r.decided_at,
            decision_note: r.decision_note,
            decided_by_display_name: r.decided_by_display_name,
        })
        .collect())
}

// ─── GET /v1/approvals/{id} ──────────────────────────────

pub async fn get_one(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<decide::ApprovalView>> {
    Ok(Json(decide::load_view(&state.db, user.user_id, id).await?))
}

// ─── Deciding ────────────────────────────────────────────

fn actor(user: &AuthUser) -> decide::Actor {
    decide::Actor {
        user_id: user.user_id,
        minted_jti: user.minted_jti,
    }
}

#[derive(Debug, Deserialize, Default)]
struct ApproveRequest {
    valid_for: Option<Validity>,
    max_requests: Option<i32>,
    /// For a reply: the hash of the draft the approver was shown.
    draft_sha256: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct DenyRequest {
    note: Option<String>,
}

// ─── POST /v1/approvals/{id}/approve ─────────────────────

pub async fn approve(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let req: ApproveRequest = optional_body(&body)?;
    let approved = decide::approve(
        &state.db,
        actor(&user),
        id,
        decide::Narrowing {
            valid_for: req.valid_for,
            max_requests: req.max_requests,
            draft_sha256: req.draft_sha256,
        },
    )
    .await?;
    Ok(Json(json!({
        "id": approved.id,
        "status": "approved",
        "invocation_id": approved.invocation_id,
        "approved_terms": approved.approved_terms,
        "consent_id": approved.consent_id,
    })))
}

// ─── POST /v1/approvals/{id}/deny ────────────────────────

pub async fn deny(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let req: DenyRequest = optional_body(&body)?;
    let denied = decide::deny(&state.db, actor(&user), id, req.note.as_deref()).await?;
    Ok(Json(json!({
        "id": denied.id,
        "status": "denied",
        "invocation_id": denied.invocation_id,
    })))
}

// ─── POST /v1/consents/{id}/revoke ───────────────────────

pub async fn revoke_consent(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    decide::revoke_consent(&state.db, actor(&user), id).await?;
    Ok(Json(json!({ "id": id, "revoked": true })))
}

// ─── GET /v1/consent-requests/{id} ───────────────────────

/// A consent request's state, for either side: never the held input.
#[derive(Debug, Serialize)]
pub struct ConsentRequestStatus {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub invocation_id: Uuid,
    pub terms: Value,
    pub approved_terms: Option<Value>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
}

pub async fn status(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ConsentRequestStatus>> {
    Ok(Json(request_status(&state, &user, id).await?))
}

/// Request `id`'s status for a member of either side (REST and MCP
/// `consent_status`).
pub async fn request_status(
    state: &RelayState,
    user: &AuthUser,
    id: Uuid,
) -> ApiResult<ConsentRequestStatus> {
    let r = sqlx::query!(
        r#"
        SELECT cr.id, cr.kind, cr.status, cr.invocation_id, cr.terms, cr.approved_terms,
               cr.expires_at, cr.created_at, cr.decided_at, cr.decision_note,
               cr.granter_agent_id, cr.grantee_agent_id,
               EXISTS (
                   SELECT 1 FROM account_memberships m
                     JOIN agents a ON a.account_id = m.account_id
                    WHERE m.user_id = $2
                      AND (a.id = cr.granter_agent_id OR a.id = cr.grantee_agent_id)
               ) AS "member!"
          FROM consent_requests cr
         WHERE cr.id = $1
        "#,
        id,
        user.user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    // Scope (migration 0027), as for invocation reads.
    let grant = crate::auth::resolve_grant(&state.db, user).await?;
    let manageable = chakramcp_shared::scope::manageable_agent_ids(&state.db, &grant).await?;
    if !r.member
        || !chakramcp_shared::scope::manages_either(
            &manageable,
            r.granter_agent_id,
            r.grantee_agent_id,
        )
    {
        return Err(ApiError::NotFound);
    }
    Ok(ConsentRequestStatus {
        id: r.id,
        kind: r.kind,
        status: r.status,
        invocation_id: r.invocation_id,
        terms: r.terms,
        approved_terms: r.approved_terms,
        expires_at: r.expires_at,
        created_at: r.created_at,
        decided_at: r.decided_at,
        decision_note: r.decision_note,
    })
}
