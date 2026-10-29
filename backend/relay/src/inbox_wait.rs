//! Waiting pulls (waking pull agents §6): `wait` on `/v1/inbox`,
//! `/v1/inbox/notices` and `/v1/inbox/calls`, and `wait_seconds` on the
//! matching MCP tools.
//!
//! A pull with nothing to claim waits on the wake hub until work is queued
//! for the agent's feed, then claims as usual. No database connection is
//! held while it waits: each claim takes one from the pool and gives it
//! back. Every REST answer carries `ChakraMCP-Wait` with the seconds this
//! server honoured, so a client can tell it from an older relay (no
//! header) and stop sleeping between polls.

use std::future::Future;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue};
use chakramcp_shared::error::ApiError;
use chakramcp_shared::wake::{Feed, Woke};
use tokio::time::Instant;
use uuid::Uuid;

use crate::state::RelayState;

/// The header that says how long this server waited (or would have).
pub const WAIT_HEADER: HeaderName = HeaderName::from_static("chakramcp-wait");

/// `ChakraMCP-Wait: <seconds>`, for a REST answer.
pub fn wait_header(honoured: u32) -> [(HeaderName, HeaderValue); 1] {
    [(WAIT_HEADER, HeaderValue::from(honoured))]
}

/// Claim with `claim`, and if nothing came back, wait up to `wait` seconds
/// (clamped to `INBOX_MAX_WAIT_SECS`) for work on `agent`'s `feed` and claim
/// again. Returns what was claimed and the seconds honoured: 0 when the
/// pull didn't wait (asked for none, every slot taken, or shutting down).
///
/// The caller has already authorized the pull and recorded a heartbeat; a
/// pull that waited records another when it returns.
pub async fn claim_waiting<T, F, Fut>(
    state: &RelayState,
    agent: Uuid,
    feed: Feed,
    wait: Option<u32>,
    mut claim: F,
) -> Result<(Vec<T>, u32), ApiError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<T>, ApiError>>,
{
    let honoured = wait.unwrap_or(0).min(state.wait.max_wait_secs);
    if honoured == 0 {
        return Ok((claim().await?, 0));
    }
    // Subscribe before the first claim, so work queued in between still
    // wakes this pull.
    let Some(mut waiter) = state.wake.try_wait(agent, feed) else {
        return Ok((claim().await?, 0));
    };
    let deadline = Instant::now() + Duration::from_secs(u64::from(honoured));
    let outcome = loop {
        let items = claim().await?;
        if !items.is_empty() {
            break (items, honoured);
        }
        match waiter.changed(deadline).await {
            // Another puller may get there first; then this one waits on.
            Woke::Changed => continue,
            Woke::Deadline => break (Vec::new(), honoured),
            // Shutting down: return without claiming again.
            Woke::Closed => break (Vec::new(), 0),
        }
    };
    drop(waiter);
    state.heartbeats.record(agent);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use chakramcp_shared::wake::{Wake, WakeSettings};

    use super::*;

    /// The load check (design §14): 2,000 pulls waiting at once, each on
    /// its own agent, all woken together. No database: the claim is a
    /// stand-in that finds work once its agent has been notified.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_thousand_waiting_pulls() {
        const PULLS: usize = 2000;
        let db = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@localhost/unused")
            .unwrap();
        let state = RelayState::new(
            db,
            crate::handlers::invoke::legacy_v01_contract_tests::config(),
        )
        .with_wake(
            Wake::new(PULLS),
            WakeSettings {
                max_wait_secs: 10,
                max_waiters: PULLS,
            },
        );
        let rss_mb = || {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(0)
                / 1024
        };
        let before = rss_mb();
        let agents: Vec<Uuid> = (0..PULLS).map(|_| Uuid::now_v7()).collect();
        let ready = Arc::new(
            (0..PULLS)
                .map(|_| std::sync::atomic::AtomicBool::new(false))
                .collect::<Vec<_>>(),
        );
        let claims = Arc::new(AtomicUsize::new(0));
        let pulls: Vec<_> = agents
            .iter()
            .enumerate()
            .map(|(i, agent)| {
                let (state, ready, claims, agent) =
                    (state.clone(), ready.clone(), claims.clone(), *agent);
                tokio::spawn(async move {
                    claim_waiting(&state, agent, Feed::Invocations, Some(10), || {
                        claims.fetch_add(1, Ordering::Relaxed);
                        let found = ready[i].load(Ordering::Acquire);
                        async move { Ok::<_, ApiError>(if found { vec![i] } else { vec![] }) }
                    })
                    .await
                    .unwrap()
                })
            })
            .collect();
        let started = Instant::now();
        while state.wake.waiting() < PULLS && started.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let settled = started.elapsed();
        assert_eq!(state.wake.waiting(), PULLS);
        let during = rss_mb();

        let woken = Instant::now();
        for (i, agent) in agents.iter().enumerate() {
            ready[i].store(true, Ordering::Release);
            state.wake.notify(*agent, Feed::Invocations);
        }
        for pull in pulls {
            let (items, waited) = pull.await.unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(waited, 10);
        }
        let all_woken = woken.elapsed();
        assert_eq!(state.wake.waiting(), 0);
        // One claim before waiting, one after the wake.
        assert_eq!(claims.load(Ordering::Relaxed), 2 * PULLS);
        println!(
            "{PULLS} waiting pulls: all waiting after {settled:?}; RSS {before} MB before, \
             {during} MB while waiting; all {PULLS} woken and answered in {all_woken:?}"
        );
        assert!(all_woken < Duration::from_secs(2), "{all_woken:?}");
    }
}
