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
//! Approving re-checks that the call is still authorized: a grant can be
//! revoked, or a capability made private, while a request waits. If it
//! isn't, the request is cancelled and the invocation rejected, with no
//! charge. Otherwise the invocation moves to `pending` (the inbox delivers
//! it) and its charge is queued, in the same transaction.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{AuthUser, InteractiveUser};
use crate::consent::approvers;
use crate::consent::terms::{ConsentTerms, Validity};
use crate::state::RelayState;

/// Longest note an approver may leave when denying.
const NOTE_MAX_CHARS: usize = 500;
/// How far back the history list goes, and how long a list may be.
const HISTORY_DAYS: i32 = 90;
const LIST_LIMIT: i64 = 200;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AgentRef {
    pub id: Uuid,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Requester {
    pub agent_id: Uuid,
    pub agent_display_name: Option<String>,
    pub account_id: Uuid,
    pub account_slug: Option<String>,
    pub account_display_name: Option<String>,
    pub verified: bool,
}

#[derive(Debug, Serialize)]
pub struct CapabilityRef {
    pub id: Uuid,
    /// `None` once the capability is deleted.
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Approver {
    pub user_id: Uuid,
    pub display_name: String,
}

/// The approval page: what's asked, by whom, and who may decide.
#[derive(Debug, Serialize)]
pub struct ApprovalView {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub invocation_id: Uuid,
    pub requester: Requester,
    pub target: AgentRef,
    pub capability: CapabilityRef,
    /// The grant's stated purpose (friend calls); the requester's own is in
    /// `terms.purpose`.
    pub grant_purpose: Option<String>,
    /// `friend` or `public`.
    pub tier: String,
    pub terms: Value,
    pub approved_terms: Option<Value>,
    /// The longest consent the publisher allows: `once`, `<n>h` or `<n>d`.
    pub max_validity: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub may_decide: bool,
    pub approvers: Vec<Approver>,
    /// The held input; only for those who may decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

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

#[derive(Debug, Deserialize, Default)]
struct ApproveRequest {
    valid_for: Option<Validity>,
    max_requests: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct DenyRequest {
    note: Option<String>,
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

/// Unrevoked, unexpired consents on agents in the user's accounts.
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
) -> ApiResult<Json<ApprovalView>> {
    let r = sqlx::query!(
        r#"
        SELECT cr.id, cr.kind, cr.status, cr.invocation_id, cr.terms, cr.approved_terms,
               cr.expires_at, cr.created_at, cr.decided_at, cr.decision_note,
               cr.capability_id, cr.granter_agent_id, cr.grantee_agent_id,
               cr.grantee_account_id,
               ga.account_id AS granter_account_id, ga.display_name AS granter_display_name,
               ea.display_name AS "grantee_display_name?",
               eacc.slug AS "grantee_account_slug?",
               eacc.display_name AS "grantee_account_display_name?",
               (eacc.verified_at IS NOT NULL) AS "grantee_verified!",
               cap.name AS "capability_name?", cap.description AS "capability_description?",
               cap.consent_approvers AS "approvers?",
               cap.consent_max_validity_secs AS "max_validity_secs?",
               i.input_preview, i.grant_id, g.purpose AS "grant_purpose?",
               EXISTS (
                   SELECT 1 FROM account_memberships m
                    WHERE m.user_id = $2 AND m.account_id = ga.account_id
               ) AS "member!"
          FROM consent_requests cr
          JOIN relay_invocations i ON i.id = cr.invocation_id
          JOIN agents ga ON ga.id = cr.granter_agent_id
          LEFT JOIN agents ea ON ea.id = cr.grantee_agent_id
          LEFT JOIN accounts eacc ON eacc.id = cr.grantee_account_id
          LEFT JOIN agent_capabilities cap ON cap.id = cr.capability_id
          LEFT JOIN grants g ON g.id = i.grant_id
         WHERE cr.id = $1
        "#,
        id,
        user.user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !r.member {
        return Err(ApiError::NotFound);
    }

    let listed = r.approvers.unwrap_or_default();
    let may_decide =
        approvers::may_decide(&state.db, user.user_id, r.granter_account_id, &listed).await?;
    let approvers = approvers::effective_approvers(&state.db, r.granter_account_id, &listed)
        .await?
        .into_iter()
        .map(|(user_id, display_name)| Approver {
            user_id,
            display_name,
        })
        .collect();
    let max_validity = Validity::from_secs(r.max_validity_secs.unwrap_or(0)).to_string();

    Ok(Json(ApprovalView {
        id: r.id,
        kind: r.kind,
        status: r.status,
        invocation_id: r.invocation_id,
        requester: Requester {
            agent_id: r.grantee_agent_id,
            agent_display_name: r.grantee_display_name,
            account_id: r.grantee_account_id,
            account_slug: r.grantee_account_slug,
            account_display_name: r.grantee_account_display_name,
            verified: r.grantee_verified,
        },
        target: AgentRef {
            id: r.granter_agent_id,
            display_name: Some(r.granter_display_name),
        },
        capability: CapabilityRef {
            id: r.capability_id,
            name: r.capability_name,
            description: r.capability_description,
        },
        grant_purpose: r.grant_purpose,
        tier: if r.grant_id.is_some() {
            "friend"
        } else {
            "public"
        }
        .into(),
        terms: r.terms,
        approved_terms: r.approved_terms,
        max_validity,
        expires_at: r.expires_at,
        created_at: r.created_at,
        decided_at: r.decided_at,
        decision_note: r.decision_note,
        may_decide,
        approvers,
        input: if may_decide { r.input_preview } else { None },
    }))
}

// ─── Deciding ────────────────────────────────────────────

/// A pending request, locked for a decision, and the decider checked.
struct Decidable {
    invocation_id: Uuid,
    terms: Value,
    capability_id: Uuid,
    granter_agent_id: Uuid,
    grantee_agent_id: Uuid,
    grantee_account_id: Uuid,
    granter_account_id: Uuid,
    max_validity_secs: i32,
}

/// Lock request `id` in `tx` and check that `user_id` may decide it now:
/// 404 unless a member of the target's account, 403 unless an approver,
/// 409 unless pending and unexpired.
async fn lock_for_decision(
    tx: &mut sqlx::PgConnection,
    state: &RelayState,
    id: Uuid,
    user_id: Uuid,
) -> ApiResult<Decidable> {
    let r = sqlx::query!(
        r#"
        SELECT cr.kind, cr.status, cr.invocation_id, cr.terms, cr.expires_at,
               cr.capability_id, cr.granter_agent_id, cr.grantee_agent_id,
               cr.grantee_account_id,
               ga.account_id AS granter_account_id,
               cap.consent_approvers AS "approvers?",
               cap.consent_max_validity_secs AS "max_validity_secs?",
               EXISTS (
                   SELECT 1 FROM account_memberships m
                    WHERE m.user_id = $2 AND m.account_id = ga.account_id
               ) AS "member!"
          FROM consent_requests cr
          JOIN agents ga ON ga.id = cr.granter_agent_id
          LEFT JOIN agent_capabilities cap ON cap.id = cr.capability_id
         WHERE cr.id = $1
           FOR UPDATE OF cr
        "#,
        id,
        user_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !r.member {
        return Err(ApiError::NotFound);
    }
    let listed = r.approvers.unwrap_or_default();
    if !approvers::may_decide(&state.db, user_id, r.granter_account_id, &listed).await? {
        return Err(ApiError::Forbidden);
    }
    if r.status != "pending" {
        return Err(ApiError::Conflict(format!(
            "this request was already {}",
            r.status
        )));
    }
    if r.expires_at <= Utc::now() {
        return Err(ApiError::Conflict("this request expired".into()));
    }
    if r.kind != "request" {
        return Err(ApiError::Conflict(
            "replies can't be confirmed here yet".into(),
        ));
    }
    Ok(Decidable {
        invocation_id: r.invocation_id,
        terms: r.terms,
        capability_id: r.capability_id,
        granter_agent_id: r.granter_agent_id,
        grantee_agent_id: r.grantee_agent_id,
        grantee_account_id: r.grantee_account_id,
        granter_account_id: r.granter_account_id,
        max_validity_secs: r.max_validity_secs.unwrap_or(0),
    })
}

/// What approving re-checks (spec §4.4): the call must still be one the
/// requester may make.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Standing {
    pub capability_exists: bool,
    pub deleted: bool,
    /// The grant's status and expiry, for friend calls.
    pub grant: Option<(String, Option<DateTime<Utc>>)>,
    pub friends: bool,
    pub public_on_network: bool,
}

/// `Err(reason)` when the call is no longer authorized.
pub(crate) fn still_authorized(s: &Standing, now: DateTime<Utc>) -> Result<(), &'static str> {
    if !s.capability_exists {
        return Err("the capability was deleted");
    }
    if s.deleted {
        return Err("an agent or account was deleted");
    }
    match &s.grant {
        Some((status, _)) if status != "active" => Err("the grant was revoked"),
        Some((_, Some(expires_at))) if *expires_at <= now => Err("the grant expired"),
        Some(_) if !s.friends => Err("the friendship ended"),
        Some(_) => Ok(()),
        None if !s.public_on_network => Err("the capability is no longer public"),
        None => Ok(()),
    }
}

async fn standing(tx: &mut sqlx::PgConnection, d: &Decidable) -> ApiResult<(Standing, String)> {
    let r = sqlx::query!(
        r#"
        SELECT i.status AS invocation_status, i.grant_id,
               (cap.id IS NOT NULL) AS "capability_exists!",
               COALESCE(cap.public_invoke, false) AS "public_invoke!",
               ga.visibility AS granter_visibility,
               (ga.tombstoned_at IS NOT NULL OR ea.tombstoned_at IS NOT NULL
                OR gacc.tombstoned_at IS NOT NULL OR eacc.tombstoned_at IS NOT NULL)
                   AS "deleted!",
               g.status AS "grant_status?", g.expires_at AS "grant_expires_at?",
               EXISTS (
                   SELECT 1 FROM friendships f
                    WHERE f.status = 'accepted'
                      AND ((f.proposer_agent_id = ga.id AND f.target_agent_id = ea.id)
                        OR (f.proposer_agent_id = ea.id AND f.target_agent_id = ga.id))
               ) AS "friends!"
          FROM relay_invocations i
          JOIN agents ga ON ga.id = $2
          JOIN agents ea ON ea.id = $3
          JOIN accounts gacc ON gacc.id = ga.account_id
          JOIN accounts eacc ON eacc.id = ea.account_id
          LEFT JOIN agent_capabilities cap ON cap.id = $4
          LEFT JOIN grants g ON g.id = i.grant_id
         WHERE i.id = $1
        "#,
        d.invocation_id,
        d.granter_agent_id,
        d.grantee_agent_id,
        d.capability_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    let grant = match (r.grant_id, r.grant_status) {
        (Some(_), Some(status)) => Some((status, r.grant_expires_at)),
        // The grant row is gone: as good as revoked.
        (Some(_), None) => Some(("deleted".to_owned(), None)),
        (None, _) => None,
    };
    Ok((
        Standing {
            capability_exists: r.capability_exists,
            deleted: r.deleted,
            grant,
            friends: r.friends,
            public_on_network: r.public_invoke && r.granter_visibility == "network",
        },
        r.invocation_status,
    ))
}

/// Settle request `id` as not delivered: the request takes `status`
/// (`denied` or `cancelled`) with `note`, and its held invocation is
/// rejected with `error`. No charge is queued.
async fn close_unapproved(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    invocation_id: Uuid,
    user_id: Option<Uuid>,
    status: &str,
    note: Option<&str>,
    error: &str,
) -> ApiResult<u64> {
    sqlx::query!(
        r#"
        UPDATE consent_requests
           SET status = $2, decided_by_user_id = $3, decided_at = now(), decision_note = $4
         WHERE id = $1 AND status = 'pending'
        "#,
        id,
        status,
        user_id,
        note,
    )
    .execute(&mut *tx)
    .await?;
    Ok(sqlx::query!(
        r#"
        UPDATE relay_invocations SET status = 'rejected', error_message = $2
         WHERE id = $1 AND status = 'awaiting_consent'
        "#,
        invocation_id,
        error,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected())
}

fn record_rejected(n: u64) {
    crate::telemetry::record_invocation_outcome(crate::telemetry::Mode::Pull, "rejected", 0, n);
}

// ─── POST /v1/approvals/{id}/approve ─────────────────────

pub async fn approve(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let req: ApproveRequest = optional_body(&body)?;
    let mut tx = state.db.begin().await?;
    let d = lock_for_decision(&mut tx, &state, id, user.user_id).await?;

    let requested: ConsentTerms = serde_json::from_value(d.terms.clone())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("stored consent terms: {e}")))?;
    let approved = requested.narrowed(req.valid_for, req.max_requests);
    approved.validate().map_err(ApiError::InvalidRequest)?;
    if !approved.within(&requested, d.max_validity_secs) {
        return Err(ApiError::InvalidRequest(
            "approved terms must be within what was asked and the capability's longest consent"
                .into(),
        ));
    }

    let (standing, invocation_status) = standing(&mut tx, &d).await?;
    if invocation_status != "awaiting_consent" {
        return Err(ApiError::Conflict(format!(
            "the invocation is already {invocation_status}"
        )));
    }
    if let Err(reason) = still_authorized(&standing, Utc::now()) {
        let rejected = close_unapproved(
            &mut tx,
            id,
            d.invocation_id,
            Some(user.user_id),
            "cancelled",
            Some(reason),
            &format!("No longer authorized: {reason}"),
        )
        .await?;
        tx.commit().await?;
        record_rejected(rejected);
        crate::consent::metrics::record("cancelled", 1);
        audit(
            &state,
            &user,
            &d,
            "consent.cancel",
            id,
            json!({ "reason": reason }),
        )
        .await;
        return Err(ApiError::Conflict(format!(
            "no longer authorized: {reason}; the request was cancelled"
        )));
    }

    let consent_id = if approved.is_standing() {
        let consent_id = Uuid::now_v7();
        sqlx::query!(
            r#"
            UPDATE consents SET revoked_at = now(), revoked_by_user_id = $3
             WHERE capability_id = $1 AND grantee_agent_id = $2 AND revoked_at IS NULL
            "#,
            d.capability_id,
            d.grantee_agent_id,
            user.user_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO consents
                (id, capability_id, granter_agent_id, grantee_agent_id, purpose,
                 valid_from, valid_until, max_requests, used, approved_by_user_id)
            VALUES ($1, $2, $3, $4, $5, now(), now() + $6::bigint * interval '1 second',
                    $7, 1, $8)
            "#,
            consent_id,
            d.capability_id,
            d.granter_agent_id,
            d.grantee_agent_id,
            approved.purpose,
            approved.valid_for.secs(),
            approved.max_requests,
            user.user_id,
        )
        .execute(&mut *tx)
        .await?;
        Some(consent_id)
    } else {
        None
    };
    let approved_terms = serde_json::to_value(&approved)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("consent terms: {e}")))?;
    sqlx::query!(
        r#"
        UPDATE consent_requests
           SET status = 'approved', decided_by_user_id = $2, decided_at = now(),
               approved_terms = $3, consent_id = $4
         WHERE id = $1 AND status = 'pending'
        "#,
        id,
        user.user_id,
        approved_terms,
        consent_id,
    )
    .execute(&mut *tx)
    .await?;
    // Release it to the inbox, and charge it now that it will be delivered.
    sqlx::query!(
        r#"
        WITH released AS (
            UPDATE relay_invocations SET status = 'pending'
             WHERE id = $1 AND status = 'awaiting_consent'
            RETURNING id
        )
        INSERT INTO credit_charge_queue (invocation_id, account_id)
        SELECT id, $2 FROM released
        "#,
        d.invocation_id,
        d.grantee_account_id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    crate::consent::metrics::record("approved", 1);
    audit(
        &state,
        &user,
        &d,
        "consent.approve",
        id,
        json!({ "terms": approved_terms, "consent_id": consent_id }),
    )
    .await;
    Ok(Json(json!({
        "id": id,
        "status": "approved",
        "invocation_id": d.invocation_id,
        "approved_terms": approved_terms,
        "consent_id": consent_id,
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
    let note = req
        .note
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty());
    if note
        .as_deref()
        .is_some_and(|n| n.chars().count() > NOTE_MAX_CHARS)
    {
        return Err(ApiError::InvalidRequest(format!(
            "note must be at most {NOTE_MAX_CHARS} characters"
        )));
    }
    let mut tx = state.db.begin().await?;
    let d = lock_for_decision(&mut tx, &state, id, user.user_id).await?;
    let error = match &note {
        Some(n) => format!("The owner declined: {n}"),
        None => "The owner declined".to_owned(),
    };
    let rejected = close_unapproved(
        &mut tx,
        id,
        d.invocation_id,
        Some(user.user_id),
        "denied",
        note.as_deref(),
        &error,
    )
    .await?;
    tx.commit().await?;

    record_rejected(rejected);
    crate::consent::metrics::record("denied", 1);
    audit(
        &state,
        &user,
        &d,
        "consent.deny",
        id,
        json!({ "note": note }),
    )
    .await;
    Ok(Json(json!({
        "id": id,
        "status": "denied",
        "invocation_id": d.invocation_id,
    })))
}

async fn audit(
    state: &RelayState,
    user: &AuthUser,
    d: &Decidable,
    action: &str,
    id: Uuid,
    metadata: Value,
) {
    crate::events::record_audit(
        &state.db,
        user,
        Some(d.granter_account_id),
        action,
        "consent_request",
        Some(id),
        Some(d.invocation_id),
        action,
        metadata,
    )
    .await;
}

// ─── POST /v1/consents/{id}/revoke ───────────────────────

pub async fn revoke_consent(
    State(state): State<RelayState>,
    InteractiveUser(user): InteractiveUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let r = sqlx::query!(
        r#"
        SELECT c.revoked_at, ga.account_id AS granter_account_id,
               cap.consent_approvers AS "approvers?",
               EXISTS (
                   SELECT 1 FROM account_memberships m
                    WHERE m.user_id = $2 AND m.account_id = ga.account_id
               ) AS "member!"
          FROM consents c
          JOIN agents ga ON ga.id = c.granter_agent_id
          LEFT JOIN agent_capabilities cap ON cap.id = c.capability_id
         WHERE c.id = $1
        "#,
        id,
        user.user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !r.member {
        return Err(ApiError::NotFound);
    }
    let listed = r.approvers.unwrap_or_default();
    if !approvers::may_decide(&state.db, user.user_id, r.granter_account_id, &listed).await? {
        return Err(ApiError::Forbidden);
    }
    let revoked = sqlx::query!(
        r#"
        UPDATE consents SET revoked_at = now(), revoked_by_user_id = $2
         WHERE id = $1 AND revoked_at IS NULL
        "#,
        id,
        user.user_id,
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    if revoked == 0 {
        return Err(ApiError::Conflict(
            "this consent was already revoked".into(),
        ));
    }
    crate::events::record_audit(
        &state.db,
        &user,
        Some(r.granter_account_id),
        "consent.revoke",
        "consent",
        Some(id),
        None,
        "consent.revoke",
        json!({}),
    )
    .await;
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
    let grant = crate::auth::resolve_grant(&state.db, &user).await?;
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
    Ok(Json(ConsentRequestStatus {
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
    }))
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn standing() -> Standing {
        Standing {
            capability_exists: true,
            deleted: false,
            grant: Some(("active".into(), None)),
            friends: true,
            public_on_network: false,
        }
    }

    #[test]
    fn still_authorized_names_what_changed() {
        let now = Utc::now();
        assert_eq!(still_authorized(&standing(), now), Ok(()));
        let cases: Vec<(Standing, &str)> = vec![
            (
                Standing {
                    capability_exists: false,
                    ..standing()
                },
                "the capability was deleted",
            ),
            (
                Standing {
                    deleted: true,
                    ..standing()
                },
                "an agent or account was deleted",
            ),
            (
                Standing {
                    grant: Some(("revoked".into(), None)),
                    ..standing()
                },
                "the grant was revoked",
            ),
            (
                Standing {
                    grant: Some(("active".into(), Some(now - chrono::Duration::hours(1)))),
                    ..standing()
                },
                "the grant expired",
            ),
            (
                Standing {
                    friends: false,
                    ..standing()
                },
                "the friendship ended",
            ),
            (
                Standing {
                    grant: None,
                    ..standing()
                },
                "the capability is no longer public",
            ),
        ];
        for (s, reason) in cases {
            assert_eq!(still_authorized(&s, now), Err(reason));
        }
        let public = Standing {
            grant: None,
            public_on_network: true,
            friends: false,
            ..standing()
        };
        assert_eq!(still_authorized(&public, now), Ok(()));
    }
}
