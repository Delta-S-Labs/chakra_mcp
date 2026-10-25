//! When what a held request depends on goes away (spec §4.6, §9).
//!
//! The consent tables have no foreign keys to the busy tables, so deleting
//! a capability or an agent cancels its pending consent requests here, in
//! the same transaction: their held invocations are rejected (no charge),
//! and their standing consents are revoked. History is kept. Revoking a
//! grant is handled in `grants::revoke`, next to its own cascade.

use sqlx::PgConnection;
use uuid::Uuid;

/// Cancel pending requests to `capability_id` and revoke its consents.
/// Returns how many held invocations were rejected.
pub async fn cancel_for_capability(
    conn: &mut PgConnection,
    capability_id: Uuid,
    reason: &str,
) -> sqlx::Result<u64> {
    let row = sqlx::query!(
        r#"
        WITH cancelled AS (
            UPDATE consent_requests
               SET status = 'cancelled', decided_at = now(), decision_note = $2
             WHERE capability_id = $1 AND status = 'pending'
            RETURNING invocation_id, kind
        ),
        rejected AS (
            UPDATE relay_invocations i
               SET status = 'rejected', error_message = $2
              FROM cancelled c
             WHERE i.id = c.invocation_id
               AND c.kind = 'request'
               AND i.status = 'awaiting_consent'
            RETURNING i.id
        ),
        revoked AS (
            UPDATE consents SET revoked_at = now()
             WHERE capability_id = $1 AND revoked_at IS NULL
        )
        SELECT (SELECT count(*) FROM cancelled) AS "cancelled!",
               (SELECT count(*) FROM rejected) AS "rejected!"
        "#,
        capability_id,
        reason,
    )
    .fetch_one(conn)
    .await?;
    Ok(record(row.cancelled, row.rejected))
}

/// Cancel pending requests to or from `agent_id` and revoke consents either
/// way. Returns how many held invocations were rejected.
pub async fn cancel_for_agent(
    conn: &mut PgConnection,
    agent_id: Uuid,
    reason: &str,
) -> sqlx::Result<u64> {
    let row = sqlx::query!(
        r#"
        WITH cancelled AS (
            UPDATE consent_requests
               SET status = 'cancelled', decided_at = now(), decision_note = $2
             WHERE (granter_agent_id = $1 OR grantee_agent_id = $1)
               AND status = 'pending'
            RETURNING invocation_id, kind
        ),
        rejected AS (
            UPDATE relay_invocations i
               SET status = 'rejected', error_message = $2
              FROM cancelled c
             WHERE i.id = c.invocation_id
               AND c.kind = 'request'
               AND i.status = 'awaiting_consent'
            RETURNING i.id
        ),
        revoked AS (
            UPDATE consents SET revoked_at = now()
             WHERE (granter_agent_id = $1 OR grantee_agent_id = $1)
               AND revoked_at IS NULL
        )
        SELECT (SELECT count(*) FROM cancelled) AS "cancelled!",
               (SELECT count(*) FROM rejected) AS "rejected!"
        "#,
        agent_id,
        reason,
    )
    .fetch_one(conn)
    .await?;
    Ok(record(row.cancelled, row.rejected))
}

/// Close a call's unfinished invocations when its content is deleted
/// (calls spec §9): their pending consent requests (a held request, or a
/// reply draft awaiting confirmation) are cancelled, so nobody can approve
/// one later; a held request is rejected without charge; any other
/// unfinished one fails. Returns how many invocations were closed.
pub async fn cancel_for_call(
    conn: &mut PgConnection,
    call_id: Uuid,
    reason: &str,
) -> sqlx::Result<u64> {
    let row = sqlx::query!(
        r#"
        WITH unfinished AS (
            SELECT id FROM relay_invocations
             WHERE call_id = $1
               AND status IN ('pending', 'in_progress', 'awaiting_consent')
        ),
        cancelled AS (
            UPDATE consent_requests c
               SET status = 'cancelled', decided_at = now(), decision_note = $2
              FROM unfinished u
             WHERE c.invocation_id = u.id AND c.status = 'pending'
            RETURNING c.invocation_id
        ),
        rejected AS (
            UPDATE relay_invocations i
               SET status = 'rejected', error_message = $2
              FROM unfinished u
             WHERE i.id = u.id AND i.status = 'awaiting_consent'
            RETURNING i.id
        ),
        failed AS (
            UPDATE relay_invocations i
               SET status = 'failed', error_message = $2,
                   elapsed_ms = LEAST(EXTRACT(EPOCH FROM (now() - i.created_at)) * 1000,
                                      2147483647)::int
              FROM unfinished u
             WHERE i.id = u.id AND i.status IN ('pending', 'in_progress')
            RETURNING i.id
        )
        SELECT (SELECT count(*) FROM cancelled) AS "cancelled!",
               (SELECT count(*) FROM rejected) AS "rejected!",
               (SELECT count(*) FROM failed) AS "failed!"
        "#,
        call_id,
        reason,
    )
    .fetch_one(conn)
    .await?;
    crate::telemetry::record_invocation_outcome(
        crate::telemetry::Mode::Pull,
        "failed",
        0,
        row.failed as u64,
    );
    Ok(record(row.cancelled, row.rejected) + row.failed as u64)
}

/// Record a cascade's metrics. They're counted when the statement succeeds;
/// a delete whose transaction then rolls back would overcount, which is rare.
fn record(cancelled: i64, rejected: i64) -> u64 {
    super::metrics::record("cancelled", cancelled as u64);
    crate::telemetry::record_invocation_outcome(
        crate::telemetry::Mode::Pull,
        "rejected",
        0,
        rejected as u64,
    );
    rejected as u64
}
