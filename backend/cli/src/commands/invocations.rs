use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List invocations (read-only — does NOT claim). Unlike `inbox pull`,
    /// this returns rows in ANY status, so use it to re-check work you
    /// claimed but haven't answered (--status in_progress) or to follow up
    /// on calls you made (--direction outbound). Wraps GET /v1/invocations.
    List {
        /// inbound = served by your agents; outbound = made by your agents.
        #[arg(long, value_parser = ["all", "inbound", "outbound"], default_value = "all")]
        direction: String,
        /// Restrict to invocations where this agent is granter or grantee.
        #[arg(long)]
        agent: Option<String>,
        /// Filter by status.
        #[arg(long, value_parser = ["pending", "in_progress", "rejected", "succeeded", "failed", "timeout"])]
        status: Option<String>,
    },
    /// Show the current state of one invocation by id.
    Get { invocation: String },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List {
            direction,
            agent,
            status,
        } => {
            let mut qs: Vec<String> = vec![format!("direction={direction}")];
            if let Some(a) = agent {
                qs.push(format!("agent_id={a}"));
            }
            if let Some(s) = status {
                qs.push(format!("status={s}"));
            }
            let path = format!("/v1/invocations?{}", qs.join("&"));
            let v: Value = api.get_relay(&path).await?;
            print(&v)
        }
        Cmd::Get { invocation } => {
            let v: Value = api
                .get_relay(&format!("/v1/invocations/{invocation}"))
                .await?;
            print(&v)
        }
    }
}
