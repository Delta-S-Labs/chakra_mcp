//! Enqueueing an invocation to a capability that asks its owner first
//! (spec §4.1).
//!
//! The trusted (REST + MCP), public and A2A paths each create invocations
//! with their own statement. For a consent-required capability all three
//! call [`enqueue_consent_required`] instead, so no path can skip the gate.
//! It is still one round trip: a single statement takes a use of a covering
//! consent if there is one, then writes the invocation (`pending` if
//! covered, `awaiting_consent` if not), its consent request, the charge
//! (covered only; a held request is charged at approval) and the notice to
//! the target agent (held only). Two callers racing for a consent's last use
//! can't both take it: the second `UPDATE` re-checks `used < max_requests`
//! after the first commits.

use chakramcp_shared::wake::{Feed, Wake};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::terms::ConsentTerms;

/// Inputs to consent-required capabilities must arrive whole, so the
/// approver sees exactly what the agent will receive (spec §3).
pub const INPUT_BYTE_LIMIT: usize = 16 * 1024;

/// Refuse an input too large to keep whole (`413 chk.request.too_large`).
pub fn check_input_size(input: &Value) -> Result<(), ApiError> {
    let len = input.to_string().len();
    if len > INPUT_BYTE_LIMIT {
        return Err(ApiError::PayloadTooLarge {
            code: "chk.request.too_large",
            message: format!(
                "this capability asks its owner before delivering requests, so its input must be \
                 at most {INPUT_BYTE_LIMIT} bytes of JSON (got {len})"
            ),
        });
    }
    Ok(())
}

/// The invocation columns the three paths share.
pub struct NewInvocation<'a> {
    pub id: Uuid,
    pub grant_id: Option<Uuid>,
    pub granter_agent_id: Uuid,
    pub grantee_agent_id: Uuid,
    pub capability_id: Uuid,
    pub capability_name: &'a str,
    pub invoked_by_user_id: Uuid,
    pub input_preview: &'a Value,
    pub api_key_id: Option<Uuid>,
    pub minted_jti: Option<Uuid>,
    pub trust_snapshot: Option<&'a Value>,
    /// The caller's account: charged, and recorded on the consent request.
    pub charge_account_id: Uuid,
    /// The call it's made in, if any (calls spec §7).
    pub call_id: Option<Uuid>,
}

/// What the capability's settings and the caller's terms add.
pub struct ConsentGate {
    /// Already capped at the capability's longest consent.
    pub terms: ConsentTerms,
    /// How long a held request waits for a decision.
    pub wait_secs: i32,
}

/// The gate for a capability, or `None` when it doesn't ask first. The
/// caller's `requested` terms are capped at the publisher's limit; with
/// none, the request is a single request.
pub fn gate_for(
    consent_required: bool,
    requested: Option<&ConsentTerms>,
    max_validity_secs: i32,
    wait_secs: i32,
) -> Option<ConsentGate> {
    consent_required.then(|| ConsentGate {
        terms: requested
            .cloned()
            .unwrap_or_default()
            .capped(max_validity_secs),
        wait_secs,
    })
}

/// Refuse terms out of range (400), before anything is written. Terms are
/// accepted on every invoke and ignored unless the capability asks first.
pub fn validate_terms(requested: Option<&ConsentTerms>) -> Result<(), ApiError> {
    match requested {
        Some(t) => t
            .validate()
            .map_err(|e| ApiError::InvalidRequest(format!("consent_terms: {e}"))),
        None => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enqueued {
    /// A standing consent covered it: queued as `pending`, as usual.
    Covered { consent_request_id: Uuid },
    /// Held as `awaiting_consent` until an approver decides.
    Held { consent_request_id: Uuid },
}

impl Enqueued {
    pub fn consent_request_id(self) -> Uuid {
        match self {
            Enqueued::Covered { consent_request_id } | Enqueued::Held { consent_request_id } => {
                consent_request_id
            }
        }
    }
}

pub async fn enqueue_consent_required(
    db: &PgPool,
    wake: &Wake,
    inv: NewInvocation<'_>,
    gate: &ConsentGate,
) -> Result<Enqueued, ApiError> {
    let consent_request_id = Uuid::now_v7();
    let notice_id = Uuid::now_v7();
    let terms = serde_json::to_value(&gate.terms)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("consent terms: {e}")))?;
    let covering_consent = sqlx::query_scalar!(
        r#"
        WITH used AS (
            UPDATE consents
               SET used = used + 1
             WHERE capability_id = $5
               AND grantee_agent_id = $4
               AND revoked_at IS NULL
               AND valid_until > now()
               AND (max_requests IS NULL OR used < max_requests)
            RETURNING id
        ),
        inv AS (
            INSERT INTO relay_invocations
                (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
                 capability_name, invoked_by_user_id, status, input_preview,
                 api_key_id, minted_jti, trust_snapshot, call_id)
            SELECT $1, $2, $3, $4, $5, $6, $7,
                   CASE WHEN EXISTS (SELECT 1 FROM used) THEN 'pending'
                        ELSE 'awaiting_consent' END,
                   $8, $9, $10, $11, $17
            RETURNING id
        ),
        charge AS (
            INSERT INTO credit_charge_queue (invocation_id, account_id)
            SELECT $1, $12 FROM used
        ),
        request AS (
            INSERT INTO consent_requests
                (id, kind, invocation_id, capability_id, granter_agent_id,
                 grantee_agent_id, grantee_account_id, status, terms, consent_id,
                 decided_at, expires_at)
            SELECT $13, 'request', $1, $5, $3, $4, $12,
                   CASE WHEN covering.id IS NULL THEN 'pending' ELSE 'approved' END,
                   $14, covering.id,
                   CASE WHEN covering.id IS NULL THEN NULL ELSE now() END,
                   now() + $15::int * interval '1 second'
              FROM (SELECT (SELECT id FROM used) AS id) covering
        ),
        notice AS (
            INSERT INTO agent_notices (id, agent_id, kind, payload)
            SELECT $16, $3, 'consent_requested',
                   jsonb_build_object('consent_request_id', $13::uuid)
             WHERE NOT EXISTS (SELECT 1 FROM used)
        )
        SELECT (SELECT id FROM used) AS "consent_id?"
        "#,
        inv.id,
        inv.grant_id,
        inv.granter_agent_id,
        inv.grantee_agent_id,
        inv.capability_id,
        inv.capability_name,
        inv.invoked_by_user_id,
        inv.input_preview,
        inv.api_key_id,
        inv.minted_jti,
        inv.trust_snapshot,
        inv.charge_account_id,
        consent_request_id,
        terms,
        gate.wait_secs,
        notice_id,
        inv.call_id,
    )
    .fetch_one(db)
    .await?;
    crate::calls::link::note(db, inv.call_id, inv.id, inv.grantee_agent_id).await;

    Ok(if covering_consent.is_some() {
        super::metrics::record("covered", 1);
        wake.notify(inv.granter_agent_id, Feed::Invocations);
        Enqueued::Covered { consent_request_id }
    } else {
        super::metrics::record("held", 1);
        // Held: the agent is told with a `consent_requested` notice.
        wake.notify(inv.granter_agent_id, Feed::Notices);
        Enqueued::Held { consent_request_id }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_size_limit() {
        assert!(check_input_size(&serde_json::json!({"a": "x".repeat(100)})).is_ok());
        let big = serde_json::json!({"a": "x".repeat(INPUT_BYTE_LIMIT)});
        match check_input_size(&big) {
            Err(ApiError::PayloadTooLarge { code, .. }) => {
                assert_eq!(code, "chk.request.too_large")
            }
            other => panic!("expected 413, got {other:?}"),
        }
    }
}
