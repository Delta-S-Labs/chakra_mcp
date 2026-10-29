//! The calls workers (spec §15): mark missed calls, end idle ones and those
//! of agents that are gone, and delete content whose TTL has run out. Every
//! relay process runs them; `SKIP LOCKED` keeps replicas apart.

use std::time::Duration;

use chakramcp_shared::wake::{Feed, Wake};
use sqlx::PgPool;
use uuid::Uuid;

use super::purge;

/// How often the workers run.
pub const INTERVAL: Duration = Duration::from_secs(30);
/// Calls handled per statement.
const BATCH: i64 = 200;
/// Calls purged per tick, each in its own transaction.
const PURGE_BATCH: i64 = 50;

/// Start the workers. Call once per process, when calls are enabled.
pub fn spawn_workers(db: PgPool, wake: Wake) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&db, &wake).await {
                tracing::warn!(error = %e, "calls worker failed; retrying next tick");
            }
        }
    });
}

/// One pass of every worker.
pub async fn tick(db: &PgPool, wake: &Wake) -> sqlx::Result<()> {
    let missed = mark_missed(db, wake).await?;
    let idle = end_idle(db, wake).await?;
    let gone = end_gone(db, wake).await?;
    let purged = purge_due(db).await?;
    if missed + idle + gone + purged > 0 {
        tracing::info!(missed, idle, gone, purged, "calls worker");
    }
    Ok(())
}

/// Calls still ringing 24 h after the invite become `missed`; the caller is
/// told.
pub async fn mark_missed(db: &PgPool, wake: &Wake) -> sqlx::Result<u64> {
    let row = sqlx::query!(
        r#"
        WITH due AS (
            SELECT id FROM calls
             WHERE state = 'ringing' AND created_at <= now() - interval '24 hours'
             ORDER BY created_at
             LIMIT $1
               FOR UPDATE SKIP LOCKED
        ),
        missed AS (
            UPDATE calls c
               SET state = 'missed', ended_at = now(), last_seq = c.last_seq + 1,
                   purge_at = now() + make_interval(secs => c.ttl_secs)
              FROM due
             WHERE c.id = due.id
            RETURNING c.id, c.last_seq, c.caller_agent_id
        ),
        entries AS (
            INSERT INTO call_entries (id, call_id, seq, kind)
            SELECT gen_random_uuid(), id, last_seq, 'missed' FROM missed
            RETURNING id, call_id
        ),
        told AS (
            INSERT INTO call_deliveries (entry_id, agent_id)
            SELECT e.id, m.caller_agent_id
              FROM entries e JOIN missed m ON m.id = e.call_id
             WHERE m.caller_agent_id IS NOT NULL
            RETURNING agent_id
        )
        SELECT (SELECT count(*) FROM missed) AS "n!",
               ARRAY(SELECT agent_id FROM told) AS "told!"
        "#,
        BATCH,
    )
    .fetch_one(db)
    .await?;
    for agent in &row.told {
        wake.notify(*agent, Feed::Calls);
    }
    Ok(row.n as u64)
}

/// Active calls quiet for `idle_secs` end (`ended_by = idle`); both agents
/// are told.
pub async fn end_idle(db: &PgPool, wake: &Wake) -> sqlx::Result<u64> {
    let row = sqlx::query!(
        r#"
        WITH due AS (
            SELECT id FROM calls
             WHERE state = 'active'
               AND COALESCE(last_message_at, answered_at, created_at)
                   + make_interval(secs => idle_secs) <= now()
             LIMIT $1
               FOR UPDATE SKIP LOCKED
        ),
        ended AS (
            UPDATE calls c
               SET state = 'ended', ended_at = now(), ended_by = 'idle',
                   last_seq = c.last_seq + 1,
                   purge_at = now() + make_interval(secs => c.ttl_secs)
              FROM due
             WHERE c.id = due.id
            RETURNING c.id, c.last_seq, c.caller_agent_id, c.callee_agent_id
        ),
        entries AS (
            INSERT INTO call_entries (id, call_id, seq, kind)
            SELECT gen_random_uuid(), id, last_seq, 'ended' FROM ended
            RETURNING id, call_id
        ),
        told AS (
            INSERT INTO call_deliveries (entry_id, agent_id)
            SELECT e.id, a.agent_id
              FROM entries e
              JOIN ended x ON x.id = e.call_id
             CROSS JOIN LATERAL (VALUES (x.caller_agent_id), (x.callee_agent_id)) AS a(agent_id)
              JOIN agents g ON g.id = a.agent_id AND g.tombstoned_at IS NULL
            RETURNING agent_id
        )
        SELECT (SELECT count(*) FROM ended) AS "n!",
               ARRAY(SELECT agent_id FROM told) AS "told!"
        "#,
        BATCH,
    )
    .fetch_one(db)
    .await?;
    for agent in &row.told {
        wake.notify(*agent, Feed::Calls);
    }
    Ok(row.n as u64)
}

/// Ringing or active calls with a party that's gone (deleted or tombstoned)
/// end (`ended_by = gone`); the party that's left is told.
pub async fn end_gone(db: &PgPool, wake: &Wake) -> sqlx::Result<u64> {
    let row = sqlx::query!(
        r#"
        WITH due AS (
            SELECT c.id FROM calls c
              LEFT JOIN agents ca ON ca.id = c.caller_agent_id
              LEFT JOIN agents cb ON cb.id = c.callee_agent_id
             WHERE c.state IN ('ringing', 'active')
               AND (ca.id IS NULL OR ca.tombstoned_at IS NOT NULL
                 OR cb.id IS NULL OR cb.tombstoned_at IS NOT NULL)
             LIMIT $1
               FOR UPDATE OF c SKIP LOCKED
        ),
        ended AS (
            UPDATE calls c
               SET state = 'ended', ended_at = now(), ended_by = 'gone',
                   last_seq = c.last_seq + 1,
                   purge_at = now() + make_interval(secs => c.ttl_secs)
              FROM due
             WHERE c.id = due.id
            RETURNING c.id, c.last_seq, c.caller_agent_id, c.callee_agent_id
        ),
        entries AS (
            INSERT INTO call_entries (id, call_id, seq, kind)
            SELECT gen_random_uuid(), id, last_seq, 'ended' FROM ended
            RETURNING id, call_id
        ),
        told AS (
            INSERT INTO call_deliveries (entry_id, agent_id)
            SELECT e.id, a.agent_id
              FROM entries e
              JOIN ended x ON x.id = e.call_id
             CROSS JOIN LATERAL (VALUES (x.caller_agent_id), (x.callee_agent_id)) AS a(agent_id)
              JOIN agents g ON g.id = a.agent_id AND g.tombstoned_at IS NULL
            RETURNING agent_id
        )
        SELECT (SELECT count(*) FROM ended) AS "n!",
               ARRAY(SELECT agent_id FROM told) AS "told!"
        "#,
        BATCH,
    )
    .fetch_one(db)
    .await?;
    for agent in &row.told {
        wake.notify(*agent, Feed::Calls);
    }
    Ok(row.n as u64)
}

/// Delete the content of calls whose TTL has run out (spec §9), each in its
/// own transaction. A call waits until its deliveries are done (or it ended
/// 24 h ago) and its invocations have finished; 7 days past `purge_at`, its
/// unfinished invocations are closed and it's purged anyway.
pub async fn purge_due(db: &PgPool) -> sqlx::Result<u64> {
    let due: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT c.id FROM calls c
         WHERE c.purged_at IS NULL AND c.purge_at <= now()
           AND (c.ended_at <= now() - interval '24 hours'
                OR NOT EXISTS (
                    SELECT 1 FROM call_deliveries d
                      JOIN call_entries e ON e.id = d.entry_id
                     WHERE e.call_id = c.id AND d.delivered_at IS NULL))
           AND (c.purge_at <= now() - interval '7 days'
                OR NOT EXISTS (
                    SELECT 1 FROM relay_invocations i
                     WHERE i.call_id = c.id
                       AND i.status IN ('pending', 'in_progress', 'awaiting_consent')))
         ORDER BY c.purge_at
         LIMIT $1
        "#,
        PURGE_BATCH,
    )
    .fetch_all(db)
    .await?;
    let mut purged = 0;
    for id in due {
        let mut tx = db.begin().await?;
        // Another process may have it, or have just purged it.
        let mine = sqlx::query_scalar!(
            "SELECT id FROM calls WHERE id = $1 AND purged_at IS NULL FOR UPDATE SKIP LOCKED",
            id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if mine.is_none() {
            continue;
        }
        let done = async {
            purge::close_invocations(&mut tx, id).await?;
            purge::purge_call(&mut tx, id).await
        }
        .await;
        match done {
            Ok(()) => {
                tx.commit().await?;
                purged += 1;
            }
            Err(e) => tracing::warn!(call_id = %id, error = %e, "purging a call failed"),
        }
    }
    Ok(purged)
}
