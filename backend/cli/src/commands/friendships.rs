use std::time::Duration;

use anyhow::Result;
use clap::Subcommand;
use serde_json::{json, Value};
use tokio::time::Instant;
use uuid::Uuid;

use crate::client::ApiClient;
use crate::print;
use crate::waiters::{self, wait_for_friendship, WaitError, MIN_POLL_INTERVAL};

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List friendships involving your agents.
    List {
        /// `all` (default), `outbound`, or `inbound`.
        #[arg(long, default_value = "all")]
        direction: String,
        /// Filter to one status.
        #[arg(long)]
        status: Option<String>,
    },
    /// Propose a friendship from one of your agents.
    Propose {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Accept a proposed friendship targeting one of your agents.
    Accept {
        id: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Reject a proposed friendship.
    Reject {
        id: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Counter a proposed friendship — opens the reverse direction with your message.
    Counter {
        id: String,
        #[arg(long)]
        message: String,
    },
    /// Cancel a proposal you sent.
    Cancel { id: String },
    /// Poll a friendship until it reaches a terminal state
    /// (`accepted | rejected | cancelled | countered | expired`) or
    /// the deadline expires. Exit codes:
    ///   * `0` accepted
    ///   * `2` timed out
    ///   * `3` rejected / cancelled / countered (terminal "no")
    ///   * `4` auth/permission
    ///   * `5` transient transport
    ///   * `6` invalid arguments
    Wait {
        /// Friendship UUID to watch.
        friendship_id: String,
        /// Seconds before we give up (default 300).
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// Seconds between polls (default 2, minimum 1).
        #[arg(long, default_value_t = 2)]
        poll_interval: u64,
        /// Emit a single JSON object on stdout (otherwise prints the
        /// full row).
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List { direction, status } => {
            let mut path = format!("/v1/friendships?direction={direction}");
            if let Some(s) = status {
                path.push_str(&format!("&status={s}"));
            }
            let v: Value = api.get_relay(&path).await?;
            print(&v)
        }
        Cmd::Propose { from, to, message } => {
            let body = json!({
                "proposer_agent_id": from,
                "target_agent_id": to,
                "proposer_message": message,
            });
            let v: Value = api.post_relay("/v1/friendships", &body).await?;
            print(&v)
        }
        Cmd::Accept { id, message } => {
            let v: Value = api
                .post_relay(
                    &format!("/v1/friendships/{id}/accept"),
                    &json!({ "response_message": message }),
                )
                .await?;
            print(&v)
        }
        Cmd::Reject { id, message } => {
            let v: Value = api
                .post_relay(
                    &format!("/v1/friendships/{id}/reject"),
                    &json!({ "response_message": message }),
                )
                .await?;
            print(&v)
        }
        Cmd::Counter { id, message } => {
            let v: Value = api
                .post_relay(
                    &format!("/v1/friendships/{id}/counter"),
                    &json!({ "proposer_message": message }),
                )
                .await?;
            print(&v)
        }
        Cmd::Cancel { id } => {
            let v: Value = api
                .post_relay(&format!("/v1/friendships/{id}/cancel"), &json!({}))
                .await?;
            print(&v)
        }
        Cmd::Wait {
            friendship_id,
            timeout,
            poll_interval,
            json,
        } => {
            let id = Uuid::parse_str(&friendship_id).map_err(|_| {
                anyhow::anyhow!(WaitError::InvalidArgs(format!(
                    "not a UUID: {friendship_id}"
                )))
            })?;
            let timeout = Duration::from_secs(timeout.max(1)).min(Duration::from_secs(86_400));
            let poll = Duration::from_secs(poll_interval).max(MIN_POLL_INTERVAL);
            let deadline = Instant::now() + timeout;

            eprintln!(
                "Waiting on friendship {id} (timeout {}s, poll {}s)…",
                timeout.as_secs(),
                poll.as_secs()
            );

            match wait_for_friendship(&api, id, deadline, poll).await {
                Ok(outcome) => {
                    if json {
                        let payload = friendship_outcome_json(&outcome);
                        print(&payload)?;
                    } else {
                        print(&outcome.row)?;
                    }
                    // accepted is the only "ok" terminal — everything
                    // else is a business "no" and should exit non-zero.
                    if outcome.status != "accepted" {
                        return Err(anyhow::anyhow!(WaitError::TerminalNo(outcome.status)));
                    }
                    Ok(())
                }
                Err(WaitError::Timeout {
                    elapsed_ms,
                    last_status,
                }) => {
                    if json {
                        let payload = json!({
                            "ok": false,
                            "friendship_id": id,
                            "status": "timeout",
                            "elapsed_ms": elapsed_ms,
                            "last_observed_status": last_status,
                        });
                        print(&payload)?;
                    }
                    Err(anyhow::anyhow!(WaitError::Timeout {
                        elapsed_ms,
                        last_status
                    }))
                }
                Err(e) => Err(anyhow::anyhow!(e)),
            }
        }
    }
}

/// Build the JSON envelope the PRD §"A) `friendships wait`" specifies.
fn friendship_outcome_json(out: &waiters::FriendshipOutcome) -> Value {
    let ok = out.status == "accepted";
    let from_agent_id = out.row.pointer("/proposer/id").cloned();
    let from_slug = waiters::pointer_str(&out.row, "/proposer/slug");
    let to_agent_id = out.row.pointer("/target/id").cloned();
    let to_slug = waiters::pointer_str(&out.row, "/target/slug");
    let decided_at = waiters::pointer_str(&out.row, "/decided_at");

    json!({
        "ok": ok,
        "friendship_id": out.friendship_id,
        "status": out.status,
        "from": {
            "agent_id": from_agent_id,
            "slug": from_slug,
        },
        "to": {
            "agent_id": to_agent_id,
            "slug": to_slug,
        },
        "decided_at": decided_at,
        "elapsed_ms": out.elapsed_ms,
    })
}
