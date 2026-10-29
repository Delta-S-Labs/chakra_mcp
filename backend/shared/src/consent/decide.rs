//! Viewing and deciding consent requests (spec §4.4, §7).
//!
//! The relay's approvals API (web sessions) and the app's built-in approval
//! page (self-hosted servers) both call these, so the two can't drift. The
//! caller authenticates the person; these check that they may decide.
//!
//! Approving re-checks that the call is still authorized: a grant can be
//! revoked, or a capability made private, while a request waits. If it
//! isn't, the request is cancelled and the invocation rejected, with no
//! charge. Otherwise the invocation moves to `pending` (the inbox delivers
//! it) and its charge is queued, in the same transaction.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::approvers;
use super::metrics;
use super::reply::{self, Draft};
use super::terms::{ConsentTerms, Validity};
use crate::error::ApiError;
use crate::wake::{Feed, Wake};

/// Longest note an approver may leave when denying.
pub const NOTE_MAX_CHARS: usize = 500;

/// Who decides: a person in their own web or page session.
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    pub user_id: Uuid,
    /// The web session's JWT id, recorded on the audit event; `None` for
    /// the built-in pages' cookie session.
    pub minted_jti: Option<Uuid>,
}

#[derive(Debug, thiserror::Error)]
pub enum DecideError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("this request was already {0}")]
    AlreadyDecided(String),
    #[error("this request expired")]
    Expired,
    /// A reply's draft was replaced after the approver loaded it.
    #[error("the draft changed, review it again")]
    DraftChanged,
    #[error("{0}")]
    BadRequest(String),
    #[error("the invocation is already {0}")]
    InvocationMoved(String),
    /// The call is no longer authorized; the request was cancelled.
    #[error("no longer authorized: {0}; the request was cancelled")]
    NoLongerAuthorized(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<DecideError> for ApiError {
    fn from(e: DecideError) -> Self {
        match e {
            DecideError::NotFound => ApiError::NotFound,
            DecideError::Forbidden => ApiError::Forbidden,
            DecideError::BadRequest(m) => ApiError::InvalidRequest(m),
            DecideError::Db(e) => ApiError::Database(e),
            DecideError::Internal(e) => ApiError::Internal(e),
            other => ApiError::Conflict(other.to_string()),
        }
    }
}

// ─── The approval view ───────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AgentRef {
    pub id: Uuid,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Requester {
    pub agent_id: Uuid,
    pub agent_display_name: Option<String>,
    pub account_id: Uuid,
    pub account_slug: Option<String>,
    pub account_display_name: Option<String>,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityRef {
    pub id: Uuid,
    /// `None` once the capability is deleted.
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Approver {
    pub user_id: Uuid,
    pub display_name: String,
}

/// The approval page: what's asked, by whom, and who may decide.
#[derive(Debug, Clone, Serialize)]
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
    /// The held input (for a reply, the original request's input); only
    /// for those who may decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    /// A reply's draft, `{status, output?, error?}`, and the hash to send
    /// when confirming it; only for those who may decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_sha256: Option<String>,
}

/// Request `id` as `user_id` may see it: `NotFound` unless a member of the
/// target agent's account.
pub async fn load_view(db: &PgPool, user_id: Uuid, id: Uuid) -> Result<ApprovalView, DecideError> {
    let r = sqlx::query!(
        r#"
        SELECT cr.id, cr.kind, cr.status, cr.invocation_id, cr.terms, cr.approved_terms,
               cr.expires_at, cr.created_at, cr.decided_at, cr.decision_note,
               cr.capability_id, cr.granter_agent_id, cr.grantee_agent_id,
               cr.grantee_account_id, cr.draft, cr.draft_sha256,
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
        user_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(DecideError::NotFound)?;
    if !r.member {
        return Err(DecideError::NotFound);
    }

    let listed = r.approvers.unwrap_or_default();
    let may_decide = approvers::may_decide(db, user_id, r.granter_account_id, &listed).await?;
    let approvers = approvers::effective_approvers(db, r.granter_account_id, &listed)
        .await?
        .into_iter()
        .map(|(user_id, display_name)| Approver {
            user_id,
            display_name,
        })
        .collect();

    Ok(ApprovalView {
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
        max_validity: Validity::from_secs(r.max_validity_secs.unwrap_or(0)).to_string(),
        expires_at: r.expires_at,
        created_at: r.created_at,
        decided_at: r.decided_at,
        decision_note: r.decision_note,
        may_decide,
        approvers,
        input: if may_decide { r.input_preview } else { None },
        draft: if may_decide { r.draft } else { None },
        draft_sha256: if may_decide { r.draft_sha256 } else { None },
    })
}

// ─── Deciding ────────────────────────────────────────────

/// A pending request, locked for a decision, and the decider checked.
struct Decidable {
    kind: String,
    draft: Option<Value>,
    draft_sha256: Option<String>,
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
/// `NotFound` unless a member of the target's account, `Forbidden` unless
/// an approver, a conflict unless pending and unexpired.
async fn lock_for_decision(
    tx: &mut PgConnection,
    db: &PgPool,
    id: Uuid,
    user_id: Uuid,
) -> Result<Decidable, DecideError> {
    let r = sqlx::query!(
        r#"
        SELECT cr.kind, cr.status, cr.invocation_id, cr.terms, cr.expires_at,
               cr.capability_id, cr.granter_agent_id, cr.grantee_agent_id,
               cr.grantee_account_id, cr.draft, cr.draft_sha256,
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
    .ok_or(DecideError::NotFound)?;
    if !r.member {
        return Err(DecideError::NotFound);
    }
    let listed = r.approvers.unwrap_or_default();
    if !approvers::may_decide(db, user_id, r.granter_account_id, &listed).await? {
        return Err(DecideError::Forbidden);
    }
    if r.status != "pending" {
        return Err(DecideError::AlreadyDecided(r.status));
    }
    if r.expires_at <= Utc::now() {
        return Err(DecideError::Expired);
    }
    Ok(Decidable {
        kind: r.kind,
        draft: r.draft,
        draft_sha256: r.draft_sha256,
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
pub struct Standing {
    pub capability_exists: bool,
    pub deleted: bool,
    /// The grant's status and expiry, for friend calls.
    pub grant: Option<(String, Option<DateTime<Utc>>)>,
    pub friends: bool,
    pub public_on_network: bool,
}

/// `Err(reason)` when the call is no longer authorized.
pub fn still_authorized(s: &Standing, now: DateTime<Utc>) -> Result<(), &'static str> {
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

async fn standing(tx: &mut PgConnection, d: &Decidable) -> Result<(Standing, String), DecideError> {
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
/// (`denied` or `cancelled`) with `note`, and its invocation ends with
/// `error`: rejected while held (`kind = request`, no charge queued), or
/// failed while its reply waited (`kind = reply`). Returns how many
/// invocations ended.
#[allow(clippy::too_many_arguments)]
async fn close_unapproved(
    tx: &mut PgConnection,
    kind: &str,
    id: Uuid,
    invocation_id: Uuid,
    user_id: Uuid,
    status: &str,
    note: Option<&str>,
    error: &str,
) -> Result<u64, DecideError> {
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
    if kind == "reply" {
        return Ok(sqlx::query!(
            r#"
            UPDATE relay_invocations
               SET status = 'failed', error_message = $2,
                   elapsed_ms = LEAST(EXTRACT(EPOCH FROM (now() - created_at)) * 1000,
                                      2147483647)::int
             WHERE id = $1 AND status = 'in_progress'
            "#,
            invocation_id,
            error,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected());
    }
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

/// How an approver narrows the requested terms; `None` keeps the request's.
/// For a reply, `draft_sha256` is the hash of the draft they were shown.
#[derive(Debug, Clone, Default)]
pub struct Narrowing {
    pub valid_for: Option<Validity>,
    pub max_requests: Option<i32>,
    pub draft_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Approved {
    pub id: Uuid,
    pub invocation_id: Uuid,
    pub approved_terms: Value,
    pub consent_id: Option<Uuid>,
}

/// Approve request `id`, as asked or narrowed.
pub async fn approve(
    db: &PgPool,
    wake: &Wake,
    actor: Actor,
    id: Uuid,
    narrowing: Narrowing,
) -> Result<Approved, DecideError> {
    let mut tx = db.begin().await?;
    let d = lock_for_decision(&mut tx, db, id, actor.user_id).await?;
    if d.kind == "reply" {
        return approve_reply(
            db,
            wake,
            tx,
            actor,
            id,
            &d,
            narrowing.draft_sha256.as_deref(),
        )
        .await;
    }

    let requested: ConsentTerms = serde_json::from_value(d.terms.clone())
        .map_err(|e| anyhow::anyhow!("stored consent terms: {e}"))?;
    let approved = requested.narrowed(narrowing.valid_for, narrowing.max_requests);
    approved.validate().map_err(DecideError::BadRequest)?;
    if !approved.within(&requested, d.max_validity_secs) {
        return Err(DecideError::BadRequest(
            "approved terms must be within what was asked and the capability's longest consent"
                .into(),
        ));
    }

    let (standing, invocation_status) = standing(&mut tx, &d).await?;
    if invocation_status != "awaiting_consent" {
        return Err(DecideError::InvocationMoved(invocation_status));
    }
    if let Err(reason) = still_authorized(&standing, Utc::now()) {
        let rejected = close_unapproved(
            &mut tx,
            "request",
            id,
            d.invocation_id,
            actor.user_id,
            "cancelled",
            Some(reason),
            &format!("No longer authorized: {reason}"),
        )
        .await?;
        tx.commit().await?;
        metrics::record_rejected(rejected);
        metrics::record("cancelled", 1);
        audit(
            db,
            actor,
            &d,
            "consent.cancel",
            id,
            json!({ "reason": reason }),
        )
        .await;
        return Err(DecideError::NoLongerAuthorized(reason));
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
            actor.user_id,
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
            actor.user_id,
        )
        .execute(&mut *tx)
        .await?;
        Some(consent_id)
    } else {
        None
    };
    let approved_terms =
        serde_json::to_value(&approved).map_err(|e| anyhow::anyhow!("consent terms: {e}"))?;
    sqlx::query!(
        r#"
        UPDATE consent_requests
           SET status = 'approved', decided_by_user_id = $2, decided_at = now(),
               approved_terms = $3, consent_id = $4
         WHERE id = $1 AND status = 'pending'
        "#,
        id,
        actor.user_id,
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
    // The request is in the agent's inbox now.
    wake.notify(d.granter_agent_id, Feed::Invocations);

    metrics::record("approved", 1);
    audit(
        db,
        actor,
        &d,
        "consent.approve",
        id,
        json!({ "terms": approved_terms, "consent_id": consent_id }),
    )
    .await;
    Ok(Approved {
        id,
        invocation_id: d.invocation_id,
        approved_terms,
        consent_id,
    })
}

/// Confirm a reply's draft: it must be the one the approver was shown,
/// then it's released as the invocation's result.
async fn approve_reply(
    db: &PgPool,
    wake: &Wake,
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Actor,
    id: Uuid,
    d: &Decidable,
    shown_sha256: Option<&str>,
) -> Result<Approved, DecideError> {
    if shown_sha256.is_none() || shown_sha256 != d.draft_sha256.as_deref() {
        return Err(DecideError::DraftChanged);
    }
    let draft: Draft = serde_json::from_value(d.draft.clone().unwrap_or(Value::Null))
        .map_err(|e| anyhow::anyhow!("stored draft: {e}"))?;
    sqlx::query!(
        r#"
        UPDATE consent_requests
           SET status = 'approved', decided_by_user_id = $2, decided_at = now()
         WHERE id = $1 AND status = 'pending'
        "#,
        id,
        actor.user_id,
    )
    .execute(&mut *tx)
    .await?;
    let elapsed = reply::release(&mut tx, d.invocation_id, &draft)
        .await?
        .ok_or_else(|| DecideError::InvocationMoved("finished".into()))?;
    reply::notify(
        &mut tx,
        d.granter_agent_id,
        "reply_confirmed",
        id,
        d.invocation_id,
    )
    .await?;
    tx.commit().await?;
    wake.notify(d.granter_agent_id, Feed::Notices);

    metrics::record_kind("reply", "approved", 1);
    metrics::record_finished(&draft.status, elapsed);
    audit(
        db,
        actor,
        d,
        "consent.approve",
        id,
        json!({ "kind": "reply", "draft_sha256": d.draft_sha256 }),
    )
    .await;
    Ok(Approved {
        id,
        invocation_id: d.invocation_id,
        approved_terms: Value::Null,
        consent_id: None,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Denied {
    pub id: Uuid,
    pub invocation_id: Uuid,
}

/// Deny request `id`, with an optional note the requester sees.
pub async fn deny(
    db: &PgPool,
    wake: &Wake,
    actor: Actor,
    id: Uuid,
    note: Option<&str>,
) -> Result<Denied, DecideError> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > NOTE_MAX_CHARS) {
        return Err(DecideError::BadRequest(format!(
            "note must be at most {NOTE_MAX_CHARS} characters"
        )));
    }
    let mut tx = db.begin().await?;
    let d = lock_for_decision(&mut tx, db, id, actor.user_id).await?;
    let is_reply = d.kind == "reply";
    let declined = if is_reply {
        "The owner declined to answer"
    } else {
        "The owner declined"
    };
    let error = match note {
        Some(n) => format!("{declined}: {n}"),
        None => declined.to_owned(),
    };
    let ended = close_unapproved(
        &mut tx,
        &d.kind,
        id,
        d.invocation_id,
        actor.user_id,
        "denied",
        note,
        &error,
    )
    .await?;
    if is_reply {
        reply::notify(
            &mut tx,
            d.granter_agent_id,
            "reply_rejected",
            id,
            d.invocation_id,
        )
        .await?;
    }
    tx.commit().await?;

    if is_reply {
        // The agent was told with a `reply_rejected` notice.
        wake.notify(d.granter_agent_id, Feed::Notices);
        metrics::record_ended("failed", ended);
        metrics::record_kind("reply", "denied", 1);
    } else {
        metrics::record_rejected(ended);
        metrics::record("denied", 1);
    }
    audit(db, actor, &d, "consent.deny", id, json!({ "note": note })).await;
    Ok(Denied {
        id,
        invocation_id: d.invocation_id,
    })
}

/// Revoke standing consent `id`: it stops covering new requests at once.
pub async fn revoke_consent(db: &PgPool, actor: Actor, id: Uuid) -> Result<(), DecideError> {
    let r = sqlx::query!(
        r#"
        SELECT ga.account_id AS granter_account_id,
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
        actor.user_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(DecideError::NotFound)?;
    if !r.member {
        return Err(DecideError::NotFound);
    }
    let listed = r.approvers.unwrap_or_default();
    if !approvers::may_decide(db, actor.user_id, r.granter_account_id, &listed).await? {
        return Err(DecideError::Forbidden);
    }
    let revoked = sqlx::query!(
        r#"
        UPDATE consents SET revoked_at = now(), revoked_by_user_id = $2
         WHERE id = $1 AND revoked_at IS NULL
        "#,
        id,
        actor.user_id,
    )
    .execute(db)
    .await?
    .rows_affected();
    if revoked == 0 {
        return Err(DecideError::AlreadyDecided("revoked".into()));
    }
    write_audit(
        db,
        actor,
        Some(r.granter_account_id),
        "consent.revoke",
        "consent",
        Some(id),
        None,
        json!({}),
    )
    .await;
    Ok(())
}

async fn audit(db: &PgPool, actor: Actor, d: &Decidable, action: &str, id: Uuid, metadata: Value) {
    write_audit(
        db,
        actor,
        Some(d.granter_account_id),
        action,
        "consent_request",
        Some(id),
        Some(d.invocation_id),
        metadata,
    )
    .await;
}

/// One audit event for a decision, as the services' own audit writers
/// record them. A failure is logged, never returned: the decision stands.
#[allow(clippy::too_many_arguments)]
async fn write_audit(
    db: &PgPool,
    actor: Actor,
    account_id: Option<Uuid>,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    target_id: Option<Uuid>,
    metadata: Value,
) {
    let res = sqlx::query!(
        r#"
        INSERT INTO audit_events
            (id, actor_user_id, account_id, api_key_id, minted_jti,
             action, resource_type, resource_id, target_id, summary, metadata)
        VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $5, $9)
        "#,
        Uuid::now_v7(),
        actor.user_id,
        account_id,
        actor.minted_jti,
        action,
        resource_type,
        resource_id,
        target_id,
        metadata,
    )
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("audit_events insert failed: {e}");
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn decide_errors_map_to_http() {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        let status = |e: DecideError| ApiError::from(e).into_response().status();
        assert_eq!(status(DecideError::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(status(DecideError::Forbidden), StatusCode::FORBIDDEN);
        assert_eq!(status(DecideError::Expired), StatusCode::CONFLICT);
        assert_eq!(
            status(DecideError::NoLongerAuthorized("the grant was revoked")),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status(DecideError::BadRequest("x".into())),
            StatusCode::BAD_REQUEST
        );
    }
}
