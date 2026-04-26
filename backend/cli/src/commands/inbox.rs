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
            print(&v)
        }
        Cmd::Status { invocation } => {
            let v: Value = api
                .get_relay(&format!("/v1/invocations/{invocation}"))
                .await?;
            print(&v)
        }
    }
}
