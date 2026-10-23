use anyhow::Result;
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::{print, read_json_arg};

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Atomically claim the oldest pending invocations targeting one of your agents.
    Pull {
        #[arg(long)]
        agent: String,
        #[arg(long, default_value_t = 25)]
        limit: i64,
    },
    /// Report a result for an invocation you claimed.
    Respond {
        invocation: String,
        #[arg(long, value_parser = ["succeeded", "failed"])]
        status: String,
        /// Output JSON — required when --status=succeeded. Use `@path` for a file.
        #[arg(long)]
        output: Option<String>,
        /// Error message — required when --status=failed.
        #[arg(long)]
        error: Option<String>,
    },
    /// Show the current state of one invocation.
    Status { invocation: String },
    /// Claim your agent's notices, oldest first; each is shown once. A
    /// `consent_requested` notice means a call is waiting for your approval:
    /// open its `approve_url`, signed in, to approve or deny it.
    Notices {
        #[arg(long)]
        agent: String,
        #[arg(long, default_value_t = 25)]
        limit: i64,
    },
}

/// A reply to a `human_in_loop` capability is a draft until an approver
/// confirms it: say so on stderr, with the link.
pub fn note_if_draft(v: &Value) {
    if v.get("status").and_then(Value::as_str) == Some("awaiting_confirmation") {
        eprintln!(
            "Saved as a draft. One of the agent's owners confirms it before the caller sees it: {}",
            v.get("approve_url").and_then(Value::as_str).unwrap_or("?")
        );
    }
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::Pull { agent, limit } => {
            let v: Value = api
                .get_relay(&format!("/v1/inbox?agent_id={agent}&limit={limit}"))
                .await?;
            print(&v)
        }
        Cmd::Respond {
            invocation,
            status,
            output,
            error,
        } => {
            // For a `human_in_loop` capability the relay keeps this as a
            // draft until one of the agent's owners confirms it on the web
            // (owner consent §5); `note_if_draft` says where.
            let mut body = json!({ "status": status });
            if status == "succeeded" {
                let out = output.ok_or_else(|| {
                    anyhow::anyhow!("--output is required when --status=succeeded")
                })?;
                body["output"] = read_json_arg(&out)?;
            }
            if status == "failed" {
                body["error"] = json!(error.unwrap_or_else(|| "failed".into()));
            }
            let v: Value = api
                .post_relay(&format!("/v1/invocations/{invocation}/result"), &body)
                .await?;
            note_if_draft(&v);
            print(&v)
        }
        Cmd::Status { invocation } => {
            let v: Value = api
                .get_relay(&format!("/v1/invocations/{invocation}"))
                .await?;
            print(&v)
        }
        Cmd::Notices { agent, limit } => {
            let v: Value = api
                .get_relay(&format!("/v1/inbox/notices?agent_id={agent}&limit={limit}"))
                .await?;
            print(&v)
        }
    }
}
