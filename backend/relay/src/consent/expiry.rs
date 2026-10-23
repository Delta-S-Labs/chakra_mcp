//! Expiring consent requests nobody decided on (spec §4.5, §9).
//!
//! Every minute, pending requests past `expires_at` become `expired`, and
//! their held invocations are rejected ("Consent request expired"), with no
//! charge. The same tick deletes agent notices older than 30 days, delivered
//! or not, so agents that never pull notices don't accumulate them. Runs in
//! every relay process; `SKIP LOCKED` keeps replicas from fighting over rows.

use std::time::Duration;

use sqlx::PgPool;

/// How often the worker runs.
pub const EXPIRY_INTERVAL: Duration = Duration::from_secs(60);

/// Requests expired per statement.
const EXPIRE_BATCH: i64 = 500;
/// Notices deleted per tick.
const NOTICE_PRUNE_BATCH: i64 = 1000;
/// How long notices are kept.
const NOTICE_RETENTION_DAYS: i32 = 30;

/// Expire one batch of overdue requests. Returns how many requests expired
/// and how many invocations that ended: held requests rejected, and calls
/// whose reply wasn't confirmed failed (with a `reply_expired` notice to
/// the agent).
pub async fn expire_once(db: &PgPool) -> sqlx::Result<(u64, u64)> {
    let row = sqlx::query!(
        r#"
        WITH due AS (
            SELECT id FROM consent_requests
             WHERE status = 'pending' AND expires_at <= now()
             ORDER BY expires_at
             LIMIT $1
             FOR UPDATE SKIP LOCKED
        ),
        expired AS (
            UPDATE consent_requests c
               SET status = 'expired', decided_at = now()
              FROM due
             WHERE c.id = due.id
            RETURNING c.invocation_id, c.kind
        ),
        rejected AS (
            UPDATE relay_invocations i
               SET status = 'rejected', error_message = 'Consent request expired'
              FROM expired e
             WHERE i.id = e.invocation_id
               AND e.kind = 'request'
               AND i.status = 'awaiting_consent'
            RETURNING i.id
        ),
        failed AS (
            UPDATE relay_invocations i
               SET status = 'failed', error_message = 'Reply not confirmed in time',
                   elapsed_ms = LEAST(EXTRACT(EPOCH FROM (now() - i.created_at)) * 1000,
                                      2147483647)::int
              FROM expired e
             WHERE i.id = e.invocation_id
               AND e.kind = 'reply'
               AND i.status = 'in_progress'
            RETURNING i.id, i.granter_agent_id
        ),
        told AS (
            INSERT INTO agent_notices (id, agent_id, kind, payload)
            SELECT gen_random_uuid(), f.granter_agent_id, 'reply_expired',
                   jsonb_build_object('invocation_id', f.id,
                                      'consent_request_id', c.id)
              FROM failed f
              JOIN consent_requests c ON c.invocation_id = f.id AND c.kind = 'reply'
             WHERE f.granter_agent_id IS NOT NULL
        )
        SELECT (SELECT count(*) FROM expired) AS "expired!",
               (SELECT count(*) FROM rejected) AS "rejected!",
               (SELECT count(*) FROM failed) AS "failed!",
               (SELECT count(*) FROM expired WHERE kind = 'reply') AS "replies!"
        "#,
        EXPIRE_BATCH,
    )
    .fetch_one(db)
    .await?;
    let (expired, rejected, failed, replies) = (
        row.expired as u64,
        row.rejected as u64,
        row.failed as u64,
        row.replies as u64,
    );
    super::metrics::record("expired", expired - replies);
    super::metrics::record_kind("reply", "expired", replies);
    crate::telemetry::record_invocation_outcome(
        crate::telemetry::Mode::Pull,
        "rejected",
        0,
        rejected,
    );
    super::metrics::record_ended("failed", failed);
    Ok((expired, rejected + failed))
}

/// Delete one batch of notices older than the retention window.
pub async fn prune_notices(db: &PgPool) -> sqlx::Result<u64> {
    Ok(sqlx::query!(
        r#"
        DELETE FROM agent_notices
         WHERE id IN (
               SELECT id FROM agent_notices
                WHERE created_at < now() - make_interval(days => $1)
                ORDER BY created_at
                LIMIT $2)
        "#,
        NOTICE_RETENTION_DAYS,
        NOTICE_PRUNE_BATCH,
    )
    .execute(db)
    .await?
    .rows_affected())
}

async fn tick(db: &PgPool) -> sqlx::Result<()> {
    // Drain a backlog in batches; one batch is the common case.
    while expire_once(db).await?.0 as i64 == EXPIRE_BATCH {}
    prune_notices(db).await?;
    let pending = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM consent_requests WHERE status = 'pending'"#
    )
    .fetch_one(db)
    .await?;
    super::metrics::set_pending(pending);
    Ok(())
}

/// Run the worker for the life of the process.
pub fn spawn_expiry_worker(db: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(EXPIRY_INTERVAL);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            if let Err(e) = tick(&db).await {
                tracing::warn!(error = %e, "consent expiry tick failed");
            }
        }
    })
}
