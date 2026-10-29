//! When each agent last pulled its inbox (house agents spec §3).
//!
//! Inbox pulls record the time in memory; a background task upserts the
//! batch into `agent_heartbeats` every 30 s. A poll never waits on the
//! database, and a delayed flush can't move a time backwards (`GREATEST`).
//!
//! A second map, never drained, keeps when each agent was last seen, so an
//! invocation can tell its caller whether the target is around (`peer`,
//! waking pull agents §8) without I/O. After each flush the task reloads
//! it from `agent_heartbeats`, so every relay copy agrees within about
//! 30 s.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// How often the recorded polls are written out.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(30);

/// Seen this recently counts as online: the directory's
/// `interval '5 minutes'`.
pub const ONLINE_WINDOW: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// How far back `peer` remembers an agent; older is "not seen".
pub const PRESENCE_DAYS: i32 = 30;

/// Polls recorded since the last flush, and when each agent was last seen.
/// Cheap to clone (shared).
#[derive(Clone, Default, Debug)]
pub struct Heartbeats {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default, Debug)]
struct Inner {
    /// Polls not written out yet.
    unflushed: HashMap<Uuid, DateTime<Utc>>,
    /// Last seen, from this process and every copy's flushes.
    presence: HashMap<Uuid, DateTime<Utc>>,
}

/// Whether an invocation's target is around, for its caller (waking pull
/// agents §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "delivery", rename_all = "lowercase")]
pub enum Peer {
    /// Pulls its inbox. `last_seen_at` is null when it hasn't pulled in
    /// the last [`PRESENCE_DAYS`] days.
    Pull {
        online: bool,
        last_seen_at: Option<DateTime<Utc>>,
    },
    /// Reached at its A2A endpoint; there's nothing to wait for.
    Push,
}

impl Peer {
    /// A sentence for a caller waiting on `name` that may not be around,
    /// e.g. "bob/venue-bot was last seen 3 hours ago, so it may not pick
    /// this up soon". `None` when it's online or a push agent. Text only,
    /// never JSON.
    pub fn hint(&self, name: &str) -> Option<String> {
        match self {
            Peer::Pull { online: true, .. } | Peer::Push => None,
            Peer::Pull {
                last_seen_at: Some(at),
                ..
            } => Some(format!(
                "{name} was last seen {}, so it may not pick this up soon.",
                ago(Utc::now() - *at)
            )),
            Peer::Pull {
                last_seen_at: None, ..
            } => Some(format!(
                "{name} hasn't been seen in the last {PRESENCE_DAYS} days, so it may not \
                 pick this up soon."
            )),
        }
    }
}

/// "12 minutes ago", "3 hours ago", "1 day ago".
fn ago(d: chrono::TimeDelta) -> String {
    let (n, unit) = if d.num_days() >= 1 {
        (d.num_days(), "day")
    } else if d.num_hours() >= 1 {
        (d.num_hours(), "hour")
    } else {
        (d.num_minutes().max(1), "minute")
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Whether an invocation in `status` is still open, so its caller is shown
/// `peer`: not settled, held for the owner included.
pub fn open(status: &str) -> bool {
    matches!(status, "pending" | "in_progress" | "awaiting_consent")
}

impl Heartbeats {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Note that `agent` pulled its inbox just now. Memory only.
    pub fn record(&self, agent: Uuid) {
        let now = Utc::now();
        let mut inner = self.lock();
        inner.unflushed.insert(agent, now);
        inner.presence.insert(agent, now);
    }

    /// When `agent` was last seen, within the last [`PRESENCE_DAYS`] days.
    pub fn last_seen(&self, agent: Uuid) -> Option<DateTime<Utc>> {
        let at = *self.lock().presence.get(&agent)?;
        (at > Utc::now() - chrono::TimeDelta::days(PRESENCE_DAYS.into())).then_some(at)
    }

    /// `peer` for an invocation of `agent`, whose `agents.mode` is `mode`.
    /// A lookup, no I/O.
    pub fn peer(&self, agent: Uuid, mode: &str) -> Peer {
        if mode == "push" {
            return Peer::Push;
        }
        let last_seen_at = self.last_seen(agent);
        Peer::Pull {
            online: last_seen_at.is_some_and(|at| Utc::now() - at < ONLINE_WINDOW),
            last_seen_at,
        }
    }

    fn drain(&self) -> HashMap<Uuid, DateTime<Utc>> {
        std::mem::take(&mut self.lock().unflushed)
    }

    /// Put back a batch that failed to write, keeping the newer time where
    /// the agent polled again meanwhile.
    fn restore(&self, batch: HashMap<Uuid, DateTime<Utc>>) {
        let mut inner = self.lock();
        for (agent, at) in batch {
            let entry = inner.unflushed.entry(agent).or_insert(at);
            if at > *entry {
                *entry = at;
            }
        }
    }

    /// Replace the last-seen map with `agent_heartbeats`' last
    /// [`PRESENCE_DAYS`] days, keeping a newer time this process recorded
    /// and hasn't flushed yet. The number of agents it now holds.
    pub async fn reload(&self, db: &PgPool) -> Result<usize, sqlx::Error> {
        let rows = sqlx::query!(
            r#"
            SELECT agent_id, last_polled_at FROM agent_heartbeats
             WHERE last_polled_at > now() - make_interval(days => $1)
            "#,
            PRESENCE_DAYS,
        )
        .fetch_all(db)
        .await?;
        let mut presence: HashMap<Uuid, DateTime<Utc>> = rows
            .into_iter()
            .map(|r| (r.agent_id, r.last_polled_at))
            .collect();
        let cutoff = Utc::now() - chrono::TimeDelta::days(PRESENCE_DAYS.into());
        let mut inner = self.lock();
        for (agent, at) in inner.presence.drain() {
            if at > cutoff {
                let entry = presence.entry(agent).or_insert(at);
                if at > *entry {
                    *entry = at;
                }
            }
        }
        inner.presence = presence;
        Ok(inner.presence.len())
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

    /// Flush, then reload the last-seen map, every [`FLUSH_INTERVAL`] for
    /// as long as the process runs.
    pub fn spawn_flusher(self, db: PgPool) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(e) = self.flush(&db).await {
                    tracing::warn!(error = %e, "agent heartbeat flush failed; will retry");
                }
                if let Err(e) = self.reload(&db).await {
                    tracing::warn!(error = %e, "agent presence reload failed; keeping the old map");
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
        let newer = *beats.lock().unflushed.get(&a).unwrap();
        beats.restore(HashMap::from([(a, old)]));
        assert_eq!(*beats.lock().unflushed.get(&a).unwrap(), newer);
        // Presence never waited on the flush.
        assert_eq!(beats.last_seen(a), Some(newer));
    }

    #[test]
    fn a_pull_makes_its_agent_online() {
        let beats = Heartbeats::default();
        let a = Uuid::now_v7();
        assert_eq!(
            beats.peer(a, "pull"),
            Peer::Pull {
                online: false,
                last_seen_at: None
            }
        );
        beats.record(a);
        assert!(matches!(
            beats.peer(a, "pull"),
            Peer::Pull {
                online: true,
                last_seen_at: Some(_)
            }
        ));
        assert_eq!(beats.peer(a, "push"), Peer::Push);
        // Draining for a flush leaves presence alone.
        beats.drain();
        assert!(beats.last_seen(a).is_some());
    }

    #[test]
    fn peer_serializes_as_the_design_says() {
        let at: DateTime<Utc> = "2026-10-28T09:12:03Z".parse().unwrap();
        assert_eq!(
            serde_json::to_value(Peer::Pull {
                online: false,
                last_seen_at: Some(at)
            })
            .unwrap(),
            serde_json::json!({ "delivery": "pull", "online": false, "last_seen_at": "2026-10-28T09:12:03Z" })
        );
        assert_eq!(
            serde_json::to_value(Peer::Pull {
                online: false,
                last_seen_at: None
            })
            .unwrap(),
            serde_json::json!({ "delivery": "pull", "online": false, "last_seen_at": null })
        );
        assert_eq!(
            serde_json::to_value(Peer::Push).unwrap(),
            serde_json::json!({ "delivery": "push" })
        );
    }

    #[test]
    fn hints_name_how_long_ago() {
        let seen = |mins: i64| Peer::Pull {
            online: false,
            last_seen_at: Some(Utc::now() - chrono::Duration::minutes(mins)),
        };
        assert_eq!(
            seen(12).hint("bob/venue-bot").unwrap(),
            "bob/venue-bot was last seen 12 minutes ago, so it may not pick this up soon."
        );
        assert_eq!(
            seen(61).hint("b").unwrap(),
            "b was last seen 1 hour ago, so it may not pick this up soon."
        );
        assert_eq!(
            seen(3 * 60 + 5).hint("b").unwrap(),
            "b was last seen 3 hours ago, so it may not pick this up soon."
        );
        assert_eq!(
            seen(2 * 24 * 60).hint("b").unwrap(),
            "b was last seen 2 days ago, so it may not pick this up soon."
        );
        assert_eq!(
            Peer::Pull {
                online: false,
                last_seen_at: None
            }
            .hint("b")
            .unwrap(),
            "b hasn't been seen in the last 30 days, so it may not pick this up soon."
        );
        assert_eq!(
            Peer::Pull {
                online: true,
                last_seen_at: Some(Utc::now())
            }
            .hint("b"),
            None
        );
        assert_eq!(Peer::Push.hint("b"), None);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn reload_takes_other_copies_times_and_keeps_newer_ones(pool: PgPool) {
        let beats = Heartbeats::default();
        let (other, mine, stale, gone) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        // Another relay copy flushed `other` and an old time for `mine`;
        // `stale` was last seen before the window.
        for (agent, ago) in [(other, "2 minutes"), (mine, "1 hour"), (stale, "31 days")] {
            sqlx::query("INSERT INTO agent_heartbeats VALUES ($1, now() - $2::interval)")
                .bind(agent)
                .bind(ago)
                .execute(&pool)
                .await
                .unwrap();
        }
        beats.record(mine);
        let mine_at = beats.last_seen(mine).unwrap();
        // An entry this copy holds that has aged out is dropped.
        beats
            .lock()
            .presence
            .insert(gone, Utc::now() - chrono::Duration::days(40));

        assert_eq!(beats.reload(&pool).await.unwrap(), 2);
        assert!(matches!(
            beats.peer(other, "pull"),
            Peer::Pull { online: true, .. }
        ));
        assert_eq!(
            beats.last_seen(mine),
            Some(mine_at),
            "memory's newer time kept"
        );
        assert_eq!(beats.last_seen(stale), None);
        assert!(!beats.lock().presence.contains_key(&gone));
    }
}
