//! Verified human-in-the-loop replies (owner consent spec §5, gate 2).
//!
//! A reply to a `human_in_loop` capability that an agent posts is stored as
//! a draft on the invocation's single `kind = 'reply'` consent request, and
//! the invocation stays `in_progress` until an approver confirms it on the
//! web (then it's released) or rejects it. A reply an approver writes in
//! the web inbox is confirmed at once. Either way, the requester knows a
//! person stood behind it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::decide::Actor;
use super::hash::sha256_jcs;
use super::metrics;
use crate::error::ApiError;
use crate::safety::{Channel, Safety};
use crate::system_one::{ComplianceChecker, Subject};
use crate::wake::{Feed, Wake};

/// A draft must arrive whole, so the approver confirms exactly what the
/// requester receives.
pub const DRAFT_BYTE_LIMIT: usize = 16 * 1024;

/// A reply as drafted, and as released.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// `succeeded` or `failed`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Draft {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// What a confirmation binds: the hash of the canonical draft. The
    /// reply token's `sha256` is the same value.
    pub fn sha256(&self) -> String {
        sha256_jcs(&self.to_value())
    }

    /// Refuse a draft too large to keep whole (`413 chk.result.too_large`).
    pub fn check_size(&self) -> Result<(), ApiError> {
        let len = self.to_value().to_string().len();
        if len > DRAFT_BYTE_LIMIT {
            return Err(ApiError::PayloadTooLarge {
                code: "chk.result.too_large",
                message: format!(
                    "a person confirms replies to this capability, so a reply must be at most \
                     {DRAFT_BYTE_LIMIT} bytes of JSON (got {len})"
                ),
            });
        }
        Ok(())
    }
}

/// The invocation a reply is for.
#[derive(Debug, Clone, Copy)]
pub struct ReplyTarget {
    pub invocation_id: Uuid,
    pub capability_id: Uuid,
    pub granter_agent_id: Uuid,
    pub grantee_agent_id: Uuid,
    pub grantee_account_id: Uuid,
    /// How long a draft waits for confirmation (the capability's wait).
    pub wait_secs: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct StoredDraft {
    pub consent_request_id: Uuid,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Store `draft` for an approver to confirm. A new draft replaces a pending
/// one (and restarts its wait); a decided reply can't be replaced.
pub async fn store_draft(
    db: &PgPool,
    t: &ReplyTarget,
    draft: &Draft,
) -> Result<StoredDraft, ApiError> {
    draft.check_size()?;
    let row = sqlx::query!(
        r#"
        INSERT INTO consent_requests
            (id, kind, invocation_id, capability_id, granter_agent_id, grantee_agent_id,
             grantee_account_id, status, terms, draft, draft_sha256, expires_at)
        VALUES ($1, 'reply', $2, $3, $4, $5, $6, 'pending', '{}'::jsonb, $7, $8,
                now() + $9::int * interval '1 second')
        ON CONFLICT (invocation_id, kind) DO UPDATE
           SET draft = EXCLUDED.draft,
               draft_sha256 = EXCLUDED.draft_sha256,
               expires_at = EXCLUDED.expires_at
         WHERE consent_requests.status = 'pending'
        RETURNING id, expires_at
        "#,
        Uuid::now_v7(),
        t.invocation_id,
        t.capability_id,
        t.granter_agent_id,
        t.grantee_agent_id,
        t.grantee_account_id,
        draft.to_value(),
        draft.sha256(),
        t.wait_secs,
    )
    .fetch_optional(db)
    .await?
    .ok_or_else(|| ApiError::Conflict("this reply was already decided".into()))?;
    metrics::record_kind("reply", "held", 1);
    Ok(StoredDraft {
        consent_request_id: row.id,
        expires_at: row.expires_at,
    })
}

/// What a reply is about, for screening it.
#[derive(Debug, Clone, Copy)]
pub struct ReplyAbout<'a> {
    pub capability_name: &'a str,
    pub capability_description: Option<&'a str>,
    /// The capability's output schema: which parts of `output` are free text.
    pub output_schema: Option<&'a Value>,
}

/// Screen a reply before anything stores or releases it (message safety
/// §5), when `checker` has safety on: the report is kept on the
/// invocation, a block refuses the reply, and otherwise it comes back as
/// the requester will see it, with what was found. Without safety the reply
/// is unchanged.
pub async fn screen_reply(
    db: &PgPool,
    checker: Option<&ComplianceChecker>,
    invocation_id: Uuid,
    about: &ReplyAbout<'_>,
    mut draft: Draft,
) -> Result<(Draft, Option<Safety>), ApiError> {
    let Some(checker) = checker.filter(|c| c.safety_enabled()) else {
        return Ok((draft, None));
    };
    let mut sent = json!({ "status": draft.status });
    if let Some(o) = &draft.output {
        sent["output"] = o.clone();
    }
    if let Some(e) = &draft.error {
        sent["error"] = json!(e);
    }
    let subject = Subject {
        capability_name: about.capability_name,
        capability_description: about
            .capability_description
            .filter(|d| !d.trim().is_empty()),
        grant_purpose: None,
        relationship: None,
        input: &sent,
    };
    let screened = checker
        .screen(Channel::Reply, &subject, "", about.output_schema, "/output")
        .await
        .map_err(|_| crate::safety::too_large_to_screen())?;
    store_reply_report(db, invocation_id, &screened.report).await?;
    if let Some(block) = screened.block {
        return Err(ApiError::Policy {
            code: block.code,
            message: block.reason,
        });
    }
    if draft.output.is_some() {
        draft.output = screened.value.get("output").cloned();
    }
    if draft.error.is_some() {
        draft.error = screened.value["error"].as_str().map(str::to_owned);
    }
    let safety = (!screened.findings.is_empty()).then_some(Safety {
        request: None,
        reply: Some(screened.findings),
    });
    Ok((draft, safety))
}

/// Keep a reply's screening report (message safety §5), drafts and blocked
/// replies included: `trust_snapshot.system_one.reply_safety`.
async fn store_reply_report(db: &PgPool, id: Uuid, report: &Value) -> Result<(), ApiError> {
    sqlx::query!(
        r#"
        UPDATE relay_invocations
           SET trust_snapshot = COALESCE(trust_snapshot, '{}'::jsonb)
               || jsonb_build_object(
                    'system_one',
                    COALESCE(trust_snapshot->'system_one', '{}'::jsonb)
                        || jsonb_build_object('reply_safety', $2::jsonb))
         WHERE id = $1
        "#,
        id,
        report,
    )
    .execute(db)
    .await?;
    Ok(())
}

/// An owner's answer, as it was released.
#[derive(Debug, Clone)]
pub struct OwnerAnswer {
    pub status: String,
    pub elapsed_ms: i32,
    /// What message safety removed or flagged in it.
    pub safety: Option<Safety>,
}

/// One of the called agent's approvers answers a call to a `human_in_loop`
/// capability on the web (waking pull agents §9): screened, stored as
/// confirmed by them and released, whether or not the agent had claimed
/// it. The caller has already checked the session is a person's.
///
/// A call nobody claimed is claimed for the owner in the same transaction,
/// and its agent, which never saw it, isn't told. A claimed one's agent
/// gets `reply_confirmed`, as when an approver confirms its draft. If any
/// step fails, nothing is committed.
pub async fn answer_as_owner(
    db: &PgPool,
    wake: &Wake,
    checker: Option<&ComplianceChecker>,
    actor: Actor,
    invocation_id: Uuid,
    draft: Draft,
) -> Result<OwnerAnswer, ApiError> {
    if !matches!(draft.status.as_str(), "succeeded" | "failed") {
        return Err(ApiError::InvalidRequest(
            "status must be 'succeeded' or 'failed'".into(),
        ));
    }
    draft.check_size()?;
    let row = sqlx::query!(
        r#"
        SELECT i.status, i.capability_id, i.granter_agent_id, i.grantee_agent_id,
               ga.account_id AS "granter_account_id?",
               ea.account_id AS "grantee_account_id?",
               c.semantics AS "semantics?",
               c.name AS "capability_name?",
               c.description AS "capability_description?",
               c.output_schema AS "output_schema?",
               c.consent_approvers AS "approvers?"
          FROM relay_invocations i
          LEFT JOIN agents ga ON ga.id = i.granter_agent_id
          LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
          LEFT JOIN agent_capabilities c ON c.id = i.capability_id
         WHERE i.id = $1
        "#,
        invocation_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let granter_account = row.granter_account_id.ok_or(ApiError::NotFound)?;
    let approvers = row.approvers.clone().unwrap_or_default();
    if !super::approvers::may_decide(db, actor.user_id, granter_account, &approvers).await? {
        return Err(ApiError::Forbidden);
    }
    if row.semantics.as_deref() != Some("human_in_loop") {
        return Err(ApiError::Conflict(
            "only calls to human_in_loop capabilities are answered by their owner; \
             others are answered by the agent that claims them"
                .into(),
        ));
    }
    match row.status.as_str() {
        "pending" | "in_progress" => {}
        "succeeded" | "failed" | "rejected" | "timeout" => return Err(already_settled()),
        other => {
            return Err(ApiError::Conflict(format!(
                "this call is {other}, so it can't be answered yet"
            )))
        }
    }
    let (
        Some(capability_id),
        Some(granter),
        Some(grantee),
        Some(grantee_account),
        Some(capability_name),
    ) = (
        row.capability_id,
        row.granter_agent_id,
        row.grantee_agent_id,
        row.grantee_account_id,
        row.capability_name.as_deref(),
    )
    else {
        return Err(ApiError::Conflict(
            "the invocation's agents or capability no longer exist".into(),
        ));
    };
    let about = ReplyAbout {
        capability_name,
        capability_description: row.capability_description.as_deref(),
        output_schema: row.output_schema.as_ref(),
    };
    let (draft, safety) = screen_reply(db, checker, invocation_id, &about, draft).await?;

    let mut tx = db.begin().await?;
    // Nobody claimed it: claim it for the owner. If its agent claimed it
    // since we looked, it's the agent's, and the agent is told below.
    let claimed_here = sqlx::query!(
        r#"
        UPDATE relay_invocations
           SET status = 'in_progress', claimed_at = now(), claimed_by_user_id = $2,
               deliveries = deliveries + 1
         WHERE id = $1 AND status = 'pending'
        "#,
        invocation_id,
        actor.user_id,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    let request_id = sqlx::query_scalar!(
        r#"
        INSERT INTO consent_requests
            (id, kind, invocation_id, capability_id, granter_agent_id, grantee_agent_id,
             grantee_account_id, status, terms, draft, draft_sha256, decided_by_user_id,
             decided_at, expires_at)
        VALUES ($1, 'reply', $2, $3, $4, $5, $6, 'approved', '{}'::jsonb, $7, $8, $9,
                now(), now())
        ON CONFLICT (invocation_id, kind) DO UPDATE
           SET draft = EXCLUDED.draft,
               draft_sha256 = EXCLUDED.draft_sha256,
               status = 'approved',
               decided_by_user_id = EXCLUDED.decided_by_user_id,
               decided_at = now()
         WHERE consent_requests.status = 'pending'
        RETURNING id
        "#,
        Uuid::now_v7(),
        invocation_id,
        capability_id,
        granter,
        grantee,
        grantee_account,
        draft.to_value(),
        draft.sha256(),
        actor.user_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::Conflict("this reply was already decided".into()))?;
    let elapsed_ms = release(&mut tx, invocation_id, &draft)
        .await?
        .ok_or_else(already_settled)?;
    if !claimed_here {
        notify(
            &mut tx,
            granter,
            "reply_confirmed",
            request_id,
            invocation_id,
        )
        .await?;
    }
    tx.commit().await?;
    if !claimed_here {
        wake.notify(granter, Feed::Notices);
    }
    metrics::record_kind("reply", "approved", 1);
    metrics::record_finished(&draft.status, elapsed_ms);
    Ok(OwnerAnswer {
        status: draft.status,
        elapsed_ms,
        safety,
    })
}

/// `409 chk.invocation.already_settled`: the call has its result, timed
/// out, or was refused (waking pull agents §11).
pub fn already_settled() -> ApiError {
    ApiError::coded(
        axum::http::StatusCode::CONFLICT,
        "chk.invocation.already_settled",
        "this invocation is already settled",
    )
}

/// Release `draft` as the invocation's result. `None` if the invocation
/// is no longer in progress.
pub(crate) async fn release(
    conn: &mut PgConnection,
    invocation_id: Uuid,
    draft: &Draft,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        UPDATE relay_invocations
           SET status = $2,
               output_preview = $3,
               error_message = $4,
               elapsed_ms = LEAST(EXTRACT(EPOCH FROM (now() - created_at)) * 1000,
                                  2147483647)::int
         WHERE id = $1 AND status = 'in_progress'
        RETURNING elapsed_ms
        "#,
        invocation_id,
        draft.status,
        draft.output.clone().unwrap_or(Value::Null),
        draft.error.as_deref(),
    )
    .fetch_optional(conn)
    .await
}

/// Tell the target agent what became of its reply (`reply_confirmed`,
/// `reply_rejected`, `reply_expired`), as a notice it pulls.
pub(crate) async fn notify(
    conn: &mut PgConnection,
    agent_id: Uuid,
    kind: &str,
    consent_request_id: Uuid,
    invocation_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO agent_notices (id, agent_id, kind, payload)
        VALUES ($1, $2, $3, $4)
        "#,
        Uuid::now_v7(),
        agent_id,
        kind,
        json!({ "consent_request_id": consent_request_id, "invocation_id": invocation_id }),
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_hash_canonically_and_have_a_size_limit() {
        let a = Draft {
            status: "succeeded".into(),
            output: Some(json!({"b": 1, "a": 2})),
            error: None,
        };
        assert_eq!(
            a.to_value(),
            json!({"status": "succeeded", "output": {"a": 2, "b": 1}})
        );
        let b: Draft =
            serde_json::from_value(json!({"output": {"a": 2, "b": 1}, "status": "succeeded"}))
                .unwrap();
        assert_eq!(a.sha256(), b.sha256());
        assert!(a.check_size().is_ok());
        let big = Draft {
            status: "succeeded".into(),
            output: Some(json!("x".repeat(DRAFT_BYTE_LIMIT))),
            error: None,
        };
        assert!(matches!(
            big.check_size(),
            Err(ApiError::PayloadTooLarge {
                code: "chk.result.too_large",
                ..
            })
        ));
    }
}
