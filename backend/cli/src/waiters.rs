//! Shared client-side polling for terminal-state friendships and
//! invocations.
//!
//! Why client-side polling and not server long-poll: the relay doesn't
//! offer one, and adding a long-poll endpoint would push significant
//! complexity into the server (connection accounting, deadlines, retry
//! semantics). For the M1 + M3 subset of issue #68 a small poll loop in
//! the CLI gives us the same UX with zero backend change.
//!
//! Jitter (±20%) on every sleep means that if a swarm of agents wake on
//! the same `friendships wait` (e.g. after a fleet restart) they
//! desynchronise on the first tick rather than hammering the relay in
//! lockstep.
//!
//! [`WaitError`] is the single error enum the new wait/ensure commands
//! return; `main.rs` downcasts on it to pick the right `exit()` code
//! (PRD §"Deterministic exit codes").

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::Rng;
use serde_json::Value;
use thiserror::Error;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

use crate::client::ApiClient;

/// Status names that `/v1/friendships/{id}` returns. Mirrors the DB
/// CHECK in migration 0004 (proposed | accepted | rejected | cancelled
/// | countered). PRD calls out an `expired` terminal state as well —
/// it's not yet in the schema, but accept it here so we won't have to
/// rev the CLI when the relay adds it.
fn is_friendship_terminal(status: &str) -> bool {
    matches!(
        status,
        "accepted" | "rejected" | "cancelled" | "countered" | "expired"
    )
}

/// Status names that `/v1/invocations/{id}` returns. Mirrors migration
/// 0007 (pending | in_progress | rejected | succeeded | failed |
/// timeout). PRD lists a `cancelled` terminal — accept it for the same
/// forward-compat reason.
fn is_invocation_terminal(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "failed" | "rejected" | "timeout" | "cancelled"
    )
}

/// Result of a successful poll loop on a friendship.
#[derive(Debug, Clone)]
pub struct FriendshipOutcome {
    pub friendship_id: Uuid,
    /// Terminal status from the relay (`accepted`, `rejected`, …).
    pub status: String,
    /// Raw row, so callers can lift `proposer`/`target`/`decided_at`
    /// fields into their JSON output without a second fetch.
    pub row: Value,
    /// Wall-clock spent inside the poll loop. The relay's
    /// `decided_at` is the source of truth for the *actual* decision
    /// time — this is just "how long did the CLI block for".
    pub elapsed_ms: u64,
}

/// Result of a successful poll loop on an invocation.
#[derive(Debug, Clone)]
pub struct InvocationOutcome {
    pub invocation_id: Uuid,
    pub status: String,
    pub row: Value,
    pub elapsed_ms: u64,
}

/// Errors the wait/ensure commands surface, each tagged with the exit
/// code main() should use (PRD §"Deterministic exit codes").
#[derive(Debug, Error)]
pub enum WaitError {
    /// `2` — deadline reached before the resource reached a terminal
    /// state. Carries the last status we saw so the caller can pass it
    /// back in the JSON envelope (`last_observed_status`).
    #[error("timed out after {elapsed_ms}ms (last status: {last_status:?})")]
    Timeout {
        elapsed_ms: u64,
        last_status: Option<String>,
    },
    /// `3` — terminal "no" (rejected / cancelled / failed). Business
    /// outcome, not an error in the transport sense.
    #[error("terminal: {0}")]
    TerminalNo(String),
    /// `4` — relay said 401 / 403. Token expired or grant revoked.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// `4` — distinct from Unauthorized so future code can branch on
    /// it, but maps to the same exit code (PRD groups them).
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// `5` — network/transport gave up after exhausting our (very
    /// modest) retry budget on a single poll attempt.
    #[error("transient: {0}")]
    Transient(String),
    /// `6` — bad CLI input (unresolvable peer, malformed UUID, …).
    #[error("invalid args: {0}")]
    InvalidArgs(String),
}

impl WaitError {
    /// Exit code per PRD §"Deterministic exit codes".
    pub fn exit_code(&self) -> i32 {
        match self {
            WaitError::Timeout { .. } => 2,
            WaitError::TerminalNo(_) => 3,
            WaitError::Unauthorized(_) | WaitError::Forbidden(_) => 4,
            WaitError::Transient(_) => 5,
            WaitError::InvalidArgs(_) => 6,
        }
    }
}

/// Sleep `base` ± 20% jitter. Bounded so the smallest legal interval
/// (1s, see `MIN_POLL_INTERVAL`) can't underflow.
async fn jittered_sleep(base: Duration) {
    let jitter_pct: f64 = rand::thread_rng().gen_range(-0.2..=0.2);
    let millis = base.as_millis() as f64 * (1.0 + jitter_pct);
    let millis = millis.max(100.0); // never sleep less than 100ms
    sleep(Duration::from_millis(millis as u64)).await;
}

/// Lowest poll interval we'll honour. Below this we'd be in
/// flood-the-relay territory.
pub const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Default poll interval (PRD §"Three locked design decisions").
/// Exposed for callers that want the same defaults as the CLI flags.
#[allow(dead_code)]
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Default timeout (PRD §"Three locked design decisions").
/// Exposed for callers that want the same defaults as the CLI flags.
#[allow(dead_code)]
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Generic poll-until-terminal loop.
///
/// `fetch` is an async closure returning the JSON row for the
/// resource. `is_terminal` decides whether a status string means "stop
/// polling". We pull the status field via `pointer("/status")`.
///
/// Returns the final row (status + body) on terminal, or a `WaitError`
/// for timeout / auth / transport-exhaust.
async fn poll_until_terminal<F, Fut>(
    fetch: F,
    is_terminal: fn(&str) -> bool,
    deadline: Instant,
    poll_interval: Duration,
) -> Result<(String, Value, u64), WaitError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Value, anyhow::Error>>,
{
    let started = Instant::now();
    let mut last_status: Option<String>;

    loop {
        match fetch().await {
            Ok(row) => {
                let status = row
                    .get("status")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| WaitError::Transient("response missing `status`".into()))?
                    .to_string();
                if is_terminal(&status) {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    return Ok((status, row, elapsed_ms));
                }
                last_status = Some(status);
            }
            Err(e) => {
                // Classify by error string — ApiClient::decode bails
                // with the HTTP status embedded in the message.
                let msg = format!("{e:#}");
                if msg.contains("401") {
                    return Err(WaitError::Unauthorized(msg));
                }
                if msg.contains("403") {
                    return Err(WaitError::Forbidden(msg));
                }
                if msg.contains("404") {
                    return Err(WaitError::InvalidArgs(msg));
                }
                // 5xx / network: treat as transient and let the next
                // tick retry. If the deadline runs out we'll surface
                // Timeout, which is the right signal for "transient
                // exhausted" too.
                last_status = Some(format!("error: {msg}"));
            }
        }

        // Did the deadline expire before we could try again?
        let now = Instant::now();
        if now >= deadline {
            return Err(WaitError::Timeout {
                elapsed_ms: started.elapsed().as_millis() as u64,
                last_status,
            });
        }
        // Don't oversleep past the deadline.
        let remaining = deadline.saturating_duration_since(now);
        let next = poll_interval.min(remaining);
        jittered_sleep(next).await;
    }
}

/// Poll `/v1/friendships/{id}` until terminal or deadline.
pub async fn wait_for_friendship(
    api: &ApiClient,
    id: Uuid,
    deadline: Instant,
    poll_interval: Duration,
) -> Result<FriendshipOutcome, WaitError> {
    let path = format!("/v1/friendships/{id}");
    let fetch = || async {
        let v: Value = api.get_relay(&path).await?;
        Ok::<_, anyhow::Error>(v)
    };
    let (status, row, elapsed_ms) =
        poll_until_terminal(fetch, is_friendship_terminal, deadline, poll_interval).await?;
    Ok(FriendshipOutcome {
        friendship_id: id,
        status,
        row,
        elapsed_ms,
    })
}

/// Poll `/v1/invocations/{id}` until terminal or deadline.
pub async fn wait_for_invocation(
    api: &ApiClient,
    id: Uuid,
    deadline: Instant,
    poll_interval: Duration,
) -> Result<InvocationOutcome, WaitError> {
    let path = format!("/v1/invocations/{id}");
    let fetch = || async {
        let v: Value = api.get_relay(&path).await?;
        Ok::<_, anyhow::Error>(v)
    };
    let (status, row, elapsed_ms) =
        poll_until_terminal(fetch, is_invocation_terminal, deadline, poll_interval).await?;
    Ok(InvocationOutcome {
        invocation_id: id,
        status,
        row,
        elapsed_ms,
    })
}

/// Format an RFC3339 string from a value's pointer, if present.
pub fn pointer_str(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr).and_then(|s| s.as_str()).map(str::to_string)
}

/// Format an `Option<DateTime<Utc>>`-like field at a pointer.
#[allow(dead_code)]
pub fn pointer_datetime(v: &Value, ptr: &str) -> Option<DateTime<Utc>> {
    pointer_str(v, ptr).and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.into()))
}

// ─── Tests ───────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use anyhow::anyhow;
    use serde_json::json;
    use tokio::time::Instant as TokioInstant;

    /// A direct test of `poll_until_terminal` — sidesteps `ApiClient`
    /// (which needs a CliConfig) by handing the loop a closure that
    /// reads from in-memory state.
    fn make_deadline(secs: u64) -> TokioInstant {
        TokioInstant::now() + Duration::from_secs(secs)
    }

    #[tokio::test(start_paused = true)]
    async fn immediate_terminal_returns_on_first_poll() {
        let fetch = || async { Ok(json!({"status": "accepted", "id": "x"})) };
        let (status, row, _ms) = poll_until_terminal(
            fetch,
            is_friendship_terminal,
            make_deadline(30),
            Duration::from_secs(1),
        )
        .await
        .expect("should succeed");
        assert_eq!(status, "accepted");
        assert_eq!(row["status"], "accepted");
    }

    #[tokio::test(start_paused = true)]
    async fn terminal_no_after_n_polls() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let fetch = move || {
            let n2 = n2.clone();
            async move {
                let i = n2.fetch_add(1, Ordering::SeqCst);
                if i < 3 {
                    Ok(json!({"status": "proposed"}))
                } else {
                    Ok(json!({"status": "rejected"}))
                }
            }
        };
        let (status, _row, _ms) = poll_until_terminal(
            fetch,
            is_friendship_terminal,
            make_deadline(120),
            Duration::from_secs(2),
        )
        .await
        .expect("should reach rejected");
        assert_eq!(status, "rejected");
        assert_eq!(n.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_triggers_timeout() {
        let fetch = || async { Ok(json!({"status": "proposed"})) };
        let err = poll_until_terminal(
            fetch,
            is_friendship_terminal,
            make_deadline(5),
            Duration::from_secs(2),
        )
        .await
        .expect_err("should time out");
        match err {
            WaitError::Timeout { last_status, .. } => {
                assert_eq!(last_status.as_deref(), Some("proposed"));
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn transient_then_success() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let fetch = move || {
            let n2 = n2.clone();
            async move {
                let i = n2.fetch_add(1, Ordering::SeqCst);
                if i == 0 {
                    Err(anyhow!("500 internal server error"))
                } else {
                    Ok(json!({"status": "succeeded"}))
                }
            }
        };
        let (status, _row, _ms) = poll_until_terminal(
            fetch,
            is_invocation_terminal,
            make_deadline(60),
            Duration::from_secs(2),
        )
        .await
        .expect("should recover");
        assert_eq!(status, "succeeded");
        assert!(n.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test(start_paused = true)]
    async fn unauthorized_short_circuits() {
        let fetch = || async { Err::<Value, _>(anyhow!("401 unauthorized")) };
        let err = poll_until_terminal(
            fetch,
            is_invocation_terminal,
            make_deadline(60),
            Duration::from_secs(2),
        )
        .await
        .expect_err("should bail on auth");
        assert!(matches!(err, WaitError::Unauthorized(_)));
        assert_eq!(err.exit_code(), 4);
    }

    #[test]
    fn exit_codes_match_prd() {
        assert_eq!(
            WaitError::Timeout {
                elapsed_ms: 0,
                last_status: None
            }
            .exit_code(),
            2
        );
        assert_eq!(WaitError::TerminalNo("rejected".into()).exit_code(), 3);
        assert_eq!(WaitError::Unauthorized("x".into()).exit_code(), 4);
        assert_eq!(WaitError::Forbidden("x".into()).exit_code(), 4);
        assert_eq!(WaitError::Transient("x".into()).exit_code(), 5);
        assert_eq!(WaitError::InvalidArgs("x".into()).exit_code(), 6);
    }

    #[test]
    fn friendship_terminal_set() {
        for s in ["accepted", "rejected", "cancelled", "countered", "expired"] {
            assert!(is_friendship_terminal(s), "{s} should be terminal");
        }
        for s in ["proposed", "", "anything-else"] {
            assert!(!is_friendship_terminal(s), "{s} should NOT be terminal");
        }
    }

    #[test]
    fn invocation_terminal_set() {
        for s in ["succeeded", "failed", "rejected", "timeout", "cancelled"] {
            assert!(is_invocation_terminal(s), "{s} should be terminal");
        }
        for s in ["pending", "in_progress", ""] {
            assert!(!is_invocation_terminal(s), "{s} should NOT be terminal");
        }
    }
}
