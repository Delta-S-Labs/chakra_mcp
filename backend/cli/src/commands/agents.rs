use anyhow::Result;
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List the agents you own.
    List,
    /// Register a new agent.
    Create {
        /// Account UUID (from `chakramcp whoami` memberships).
        #[arg(long)]
        account: String,
        /// Slug — unique within the account, ascii alphanumeric / dash / underscore.
        #[arg(long)]
        slug: String,
        /// Display name.
        #[arg(long)]
        name: String,
        /// One-line description.
        #[arg(long, default_value = "")]
        description: String,
        /// `private` (default) or `network`.
        #[arg(long, default_value = "private")]
        visibility: String,
        /// A2A canonical Agent Card URL for **push-mode** agents
        /// (those with a public A2A endpoint). When supplied, the
        /// relay registers the agent in push mode and starts
        /// fetching + caching its upstream card. When omitted, the
        /// agent registers in pull mode and is expected to run
        /// `inbox.serve()` against the relay (the default for
        /// laptop / sandboxed agents).
        #[arg(long)]
        agent_card_url: Option<String>,
    },
    /// Fetch one agent by id.
    Get {
        id: String,
    },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List => {
            let agents: Value = api.get_relay("/v1/agents").await?;
            print(&agents)
        }
        Cmd::Create {
            account,
            slug,
            name,
            description,
            visibility,
            agent_card_url,
        } => {
            let mut body = json!({
                "account_id": account,
                "slug": slug,
                "display_name": name,
                "description": description,
                "visibility": visibility,
            });
            // Push mode is opt-in by passing --agent-card-url; the
            // server picks the mode from the URL's presence (see
            // backend/relay/src/handlers/agents.rs::create), and the
            // DB CHECK enforces the card-or-pull invariant.
            if let Some(url) = agent_card_url {
                body["agent_card_url"] = json!(url);
            }
            let agent: Value = api.post_relay("/v1/agents", &body).await?;
            print(&agent)
        }
        Cmd::Get { id } => {
            let agent: Value = api.get_relay(&format!("/v1/agents/{id}")).await?;
            print(&agent)
        }
    }
}
