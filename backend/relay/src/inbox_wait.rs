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
