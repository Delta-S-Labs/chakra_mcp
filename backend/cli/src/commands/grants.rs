use anyhow::Result;
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List grants involving your agents.
    List {
        #[arg(long, default_value = "all")]
        direction: String,
        #[arg(long)]
        status: Option<String>,
    },
    /// Issue a grant — requires an accepted friendship between the two agents.
    Create {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        capability: String,
    },
    /// Revoke a grant you issued.
    Revoke {
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List { direction, status } => {
            let mut path = format!("/v1/grants?direction={direction}");
            if let Some(s) = status {
                path.push_str(&format!("&status={s}"));
            }
            let v: Value = api.get_relay(&path).await?;
            print(&v)
        }
        Cmd::Create { from, to, capability } => {
            let body = json!({
                "granter_agent_id": from,
                "grantee_agent_id": to,
                "capability_id": capability,
            });
            let v: Value = api.post_relay("/v1/grants", &body).await?;
            print(&v)
        }
        Cmd::Revoke { id, reason } => {
            let v: Value = api
                .post_relay(&format!("/v1/grants/{id}/revoke"), &json!({ "reason": reason }))
                .await?;
            print(&v)
        }
    }
}
