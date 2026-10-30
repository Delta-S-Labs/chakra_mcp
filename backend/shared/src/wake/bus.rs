//! Carrying wakes between processes (design §5): the relay may run several
//! copies, and the app's built-in approval page queues work too.
//!
//! The sender drains the hub's outbound keys and publishes them with
//! `pg_notify` in batches; the listener signals local waiters for keys that
//! other processes published. Both run in the background, so a slow or
//! broken database connection never touches a request. If the listener's
//! connection drops, every local waiter is woken to claim again, since
//! messages sent meanwhile are lost; a waiter always returns at its deadline
//! anyway, so the worst case is the old polling delay.

use std::time::Duration;

use metrics::counter;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Feed, Key, Wake};
use crate::telemetry::names;

/// The Postgres channel.
pub const CHANNEL: &str = "chakramcp_wake";

/// Payloads stay under this, below Postgres's 8,000-byte limit.
const MAX_PAYLOAD: usize = 7 * 1024;

/// How long the sender lets keys gather before publishing.
const GATHER: Duration = Duration::from_millis(50);

/// How a reconnect's wake-everyone is spread out, so up to thousands of
/// waiters don't all claim against the pool at once.
const SPREAD_SLICES: usize = 20;
const SPREAD_GAP: Duration = Duration::from_millis(50);

#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    /// The sending process.
    o: Uuid,
    /// The sender runs no wake-URL dispatcher (the standalone app), so the
    /// receivers' dispatchers ping for these (design §5, §7).
    #[serde(default)]
    d: bool,
    /// `[agent, feed code]` pairs.
    w: Vec<(Uuid, String)>,
}

/// Start the sender and the listener for `wake`.
pub fn spawn(pool: PgPool, wake: Wake) {
    tokio::spawn(send_loop(pool.clone(), wake.clone()));
    tokio::spawn(listen_loop(pool, wake));
}

async fn send_loop(pool: PgPool, wake: Wake) {
    loop {
        wake.0.outbound_ready.notified().await;
        tokio::time::sleep(GATHER).await;
        let keys = wake.drain_outbound();
        if keys.is_empty() {
            continue;
        }
        let mut batches = payloads(wake.process(), !wake.dispatching(), &keys).into_iter();
        while let Some((payload, batch)) = batches.next() {
            if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
                .bind(CHANNEL)
                .bind(&payload)
                .execute(&pool)
                .await
            {
                tracing::warn!(error = %e, "couldn't publish wakes; retrying");
                wake.requeue(batch);
                wake.requeue(batches.flat_map(|(_, rest)| rest));
                tokio::time::sleep(Duration::from_secs(1)).await;
                wake.0.outbound_ready.notify_one();
                break;
            }
        }
    }
}

/// Split `keys` into payloads under [`MAX_PAYLOAD`], each with its keys.
/// `undispatched`: this process runs no wake-URL dispatcher.
fn payloads(process: Uuid, undispatched: bool, keys: &[Key]) -> Vec<(String, Vec<Key>)> {
    let mut out = Vec::new();
    let mut batch: Vec<Key> = Vec::new();
    let encode = |batch: &[Key]| {
        serde_json::to_string(&Payload {
            o: process,
            d: undispatched,
            w: batch
                .iter()
                .map(|(agent, feed)| (*agent, feed.code().to_owned()))
                .collect(),
        })
        .expect("payload serializes")
    };
    for key in keys {
        batch.push(*key);
        if encode(&batch).len() > MAX_PAYLOAD {
            let last = batch.pop().expect("just pushed");
            out.push((encode(&batch), std::mem::take(&mut batch)));
            batch.push(last);
        }
    }
    if !batch.is_empty() {
        out.push((encode(&batch), batch));
    }
    out
}

async fn listen_loop(pool: PgPool, wake: Wake) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut listener = match connect(&pool).await {
            Ok(listener) => {
                backoff = Duration::from_secs(1);
                listener
            }
            Err(e) => {
                tracing::warn!(error = %e, "can't listen for wakes from other processes; retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        loop {
            match listener.try_recv().await {
                Ok(Some(message)) => receive(&wake, message.payload()),
                Ok(None) => {
                    // The connection dropped and is being re-established:
                    // anything sent meanwhile is lost, so everyone checks.
                    counter!(names::WAKE_LISTENER_RECONNECTS_TOTAL).increment(1);
                    tracing::info!("wake listener reconnected; waking local waiters");
                    tokio::spawn(wake_everyone(wake.clone()));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "wake listener failed; reconnecting");
                    counter!(names::WAKE_LISTENER_RECONNECTS_TOTAL).increment(1);
                    tokio::spawn(wake_everyone(wake.clone()));
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                    break;
                }
            }
        }
    }
}

async fn connect(pool: &PgPool) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(CHANNEL).await?;
    Ok(listener)
}

/// Signal the local waiters for another process's keys.
fn receive(wake: &Wake, raw: &str) {
    let payload: Payload = match serde_json::from_str(raw) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error = %e, "ignoring a malformed wake payload");
            return;
        }
    };
    if payload.o == wake.process() {
        return;
    }
    for (agent, code) in payload.w {
        if let Some(feed) = Feed::from_code(&code) {
            wake.signal_local(agent, feed);
            // Wakes from a process with a dispatcher were dispatched there.
            if payload.d {
                wake.queue_dispatch(agent);
            }
        }
    }
}

/// Wake every local waiter, spread over about a second.
async fn wake_everyone(wake: Wake) {
    let keys = wake.waited_keys();
    let per_slice = keys.len().div_ceil(SPREAD_SLICES).max(1);
    for slice in keys.chunks(per_slice) {
        for (agent, feed) in slice {
            wake.signal_local(*agent, *feed);
        }
        tokio::time::sleep(SPREAD_GAP).await;
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Instant;

    use super::super::Woke;
    use super::*;

    #[test]
    fn payloads_stay_under_the_limit_and_keep_every_key() {
        let keys: Vec<Key> = (0..500)
            .map(|i| {
                let feed = [Feed::Invocations, Feed::Notices, Feed::Calls][i % 3];
                (Uuid::now_v7(), feed)
            })
            .collect();
        let batches = payloads(Uuid::now_v7(), false, &keys);
        assert!(batches.len() > 1);
        let mut seen = Vec::new();
        for (payload, batch) in &batches {
            assert!(payload.len() <= MAX_PAYLOAD, "{} bytes", payload.len());
            let parsed: Payload = serde_json::from_str(payload).unwrap();
            assert_eq!(parsed.w.len(), batch.len());
            seen.extend(batch.iter().copied());
        }
        assert_eq!(seen, keys);
    }

    #[test]
    fn a_process_ignores_its_own_messages_and_bad_ones() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        let waiter = wake.try_wait(agent, Feed::Invocations).unwrap();
        let mine = payloads(wake.process(), false, &[(agent, Feed::Invocations)]);
        receive(&wake, &mine[0].0);
        receive(&wake, "not json");
        receive(
            &wake,
            r#"{"o":"00000000-0000-0000-0000-000000000000","w":[["x","i"]]}"#,
        );
        assert!(!waiter.changes.as_ref().unwrap().has_changed().unwrap());
        let theirs = payloads(Uuid::now_v7(), false, &[(agent, Feed::Invocations)]);
        receive(&wake, &theirs[0].0);
        assert!(waiter.changes.as_ref().unwrap().has_changed().unwrap());
    }

    #[test]
    fn only_wakes_from_a_process_without_a_dispatcher_are_dispatched_here() {
        let wake = Wake::new(10);
        let (agent, other) = (Uuid::now_v7(), Uuid::now_v7());
        // Not dispatching yet: nothing is gathered, local or remote.
        wake.notify(agent, Feed::Invocations);
        assert!(wake.take_dispatch().is_empty());

        wake.start_dispatching();
        wake.notify(agent, Feed::Notices);
        assert_eq!(wake.take_dispatch(), [agent]);
        // Another relay dispatched its own wake: not again here.
        let relays = payloads(Uuid::now_v7(), false, &[(other, Feed::Invocations)]);
        receive(&wake, &relays[0].0);
        assert!(wake.take_dispatch().is_empty());
        // The standalone app has no dispatcher: its wakes are ours to ping.
        let app = payloads(Uuid::now_v7(), true, &[(other, Feed::Invocations)]);
        assert!(app[0].0.contains(r#""d":true"#));
        receive(&wake, &app[0].0);
        assert_eq!(wake.take_dispatch(), [other]);
        // Payloads from before `d` existed parse, and aren't dispatched.
        receive(
            &wake,
            &format!(r#"{{"o":"{}","w":[["{other}","i"]]}}"#, Uuid::now_v7()),
        );
        assert!(wake.take_dispatch().is_empty());
    }

    #[sqlx::test(migrations = false)]
    async fn a_notify_in_one_process_wakes_a_waiter_in_another(pool: PgPool) {
        let a = Wake::new(10);
        let b = Wake::new(10);
        spawn(pool.clone(), a.clone());
        spawn(pool.clone(), b.clone());
        let agent = Uuid::now_v7();
        let mut waiter = b.try_wait(agent, Feed::Calls).unwrap();
        // Give both listeners time to subscribe.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut woke = Woke::Deadline;
        while Instant::now() < deadline {
            a.notify(agent, Feed::Calls);
            woke = waiter
                .changed(Instant::now() + Duration::from_millis(300))
                .await;
            if woke == Woke::Changed {
                break;
            }
        }
        assert_eq!(woke, Woke::Changed);
    }
}
