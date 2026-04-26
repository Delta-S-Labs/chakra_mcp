use anyhow::Result;
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

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
    }
}
