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

/// An approver's own reply (in the web inbox): store it as confirmed by
/// them and release it, in one transaction. Returns the elapsed time.
pub async fn confirm_now(
    db: &PgPool,
    actor: Actor,
    t: &ReplyTarget,
    draft: &Draft,
) -> Result<i32, ApiError> {
    draft.check_size()?;
    let mut tx = db.begin().await?;
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
        t.invocation_id,
        t.capability_id,
        t.granter_agent_id,
        t.grantee_agent_id,
        t.grantee_account_id,
        draft.to_value(),
        draft.sha256(),
        actor.user_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::Conflict("this reply was already decided".into()))?;
    let elapsed = release(&mut tx, t.invocation_id, draft)
        .await?
        .ok_or_else(|| ApiError::Conflict("the invocation is no longer in progress".into()))?;
    notify(
        &mut tx,
        t.granter_agent_id,
        "reply_confirmed",
        request_id,
        t.invocation_id,
    )
    .await?;
    tx.commit().await?;
    metrics::record_kind("reply", "approved", 1);
    metrics::record_finished(&draft.status, elapsed);
    Ok(elapsed)
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
