//! When each agent last pulled its inbox (house agents spec §3).
//!
//! Inbox pulls record the time in memory; a background task upserts the
//! batch into `agent_heartbeats` every 30 s. A poll never waits on the
//! database, and a delayed flush can't move a time backwards (`GREATEST`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// How often the recorded polls are written out.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(30);

/// Polls recorded since the last flush. Cheap to clone (shared).
#[derive(Clone, Default, Debug)]
pub struct Heartbeats {
    seen: Arc<Mutex<HashMap<Uuid, DateTime<Utc>>>>,
}

impl Heartbeats {
    /// Note that `agent` pulled its inbox just now. Memory only.
    pub fn record(&self, agent: Uuid) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(agent, Utc::now());
    }

    fn drain(&self) -> HashMap<Uuid, DateTime<Utc>> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Put back a batch that failed to write, keeping the newer time where
    /// the agent polled again meanwhile.
    fn restore(&self, batch: HashMap<Uuid, DateTime<Utc>>) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        for (agent, at) in batch {
            let entry = seen.entry(agent).or_insert(at);
            if at > *entry {
                *entry = at;
            }
        }
    }

    /// Write everything recorded since the last flush in one upsert; the
    /// number of agents written. On error the batch is kept for next time.
    pub async fn flush(&self, db: &PgPool) -> Result<u64, sqlx::Error> {
        let batch = self.drain();
        if batch.is_empty() {
            return Ok(0);
        }
        let (agents, times): (Vec<Uuid>, Vec<DateTime<Utc>>) =
            batch.iter().map(|(a, t)| (*a, *t)).unzip();
        let result = sqlx::query!(
            r#"
            INSERT INTO agent_heartbeats (agent_id, last_polled_at)
            SELECT * FROM unnest($1::uuid[], $2::timestamptz[])
            ON CONFLICT (agent_id) DO UPDATE
               SET last_polled_at = GREATEST(agent_heartbeats.last_polled_at,
                                             EXCLUDED.last_polled_at)
            "#,
            &agents,
            &times,
        )
        .execute(db)
        .await;
        match result {
            Ok(done) => Ok(done.rows_affected()),
            Err(e) => {
                self.restore(batch);
                Err(e)
            }
        }
    }

    /// Flush every [`FLUSH_INTERVAL`] for as long as the process runs.
    pub fn spawn_flusher(self, db: PgPool) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(e) = self.flush(&db).await {
                    tracing::warn!(error = %e, "agent heartbeat flush failed; will retry");
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn last_polled(pool: &PgPool, agent: Uuid) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT last_polled_at FROM agent_heartbeats WHERE agent_id = $1")
            .bind(agent)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn polls_are_written_in_one_batch_and_never_move_backwards(pool: PgPool) {
        let beats = Heartbeats::default();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        beats.record(a);
        beats.record(b);
        beats.record(a);
        assert_eq!(beats.flush(&pool).await.unwrap(), 2);
        assert_eq!(beats.flush(&pool).await.unwrap(), 0, "nothing new");
        let first = last_polled(&pool, a).await.unwrap();

        // A stale time arriving late doesn't overwrite a newer one.
        sqlx::query("UPDATE agent_heartbeats SET last_polled_at = now() + interval '1 hour' WHERE agent_id = $1")
            .bind(a)
            .execute(&pool)
            .await
            .unwrap();
        beats.record(a);
        beats.flush(&pool).await.unwrap();
        assert!(last_polled(&pool, a).await.unwrap() > first + chrono::Duration::minutes(30));
        assert!(last_polled(&pool, b).await.is_some());
    }

    #[test]
    fn a_failed_batch_is_kept_with_the_newest_times() {
        let beats = Heartbeats::default();
        let a = Uuid::now_v7();
        let old = Utc::now() - chrono::Duration::minutes(5);
        beats.record(a);
        let newer = *beats.seen.lock().unwrap().get(&a).unwrap();
        beats.restore(HashMap::from([(a, old)]));
        assert_eq!(*beats.seen.lock().unwrap().get(&a).unwrap(), newer);
    }
}
