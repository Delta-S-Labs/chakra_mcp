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
    Get { id: String },
    /// Change one of your agents: its name, description, visibility, or
    /// who may call it.
    Update {
        id: String,
        /// Display name.
        #[arg(long)]
        name: Option<String>,
        /// One-line description.
        #[arg(long)]
        description: Option<String>,
        /// `private` or `network`.
        #[arg(long)]
        visibility: Option<String>,
        /// Who may call this agent: its friends (the default), anyone, or nobody.
        #[arg(long, value_parser = ["friends", "anyone", "nobody"])]
        accept_calls: Option<String>,
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
        Cmd::Update {
            id,
            name,
            description,
            visibility,
            accept_calls,
        } => {
            let body = update_body(name, description, visibility, accept_calls)?;
            let agent: Value = api.patch_relay(&format!("/v1/agents/{id}"), &body).await?;
            print(&agent)
        }
    }
}

fn update_body(
    name: Option<String>,
    description: Option<String>,
    visibility: Option<String>,
    accept_calls: Option<String>,
) -> Result<Value> {
    let mut body = json!({});
    for (key, value) in [
        ("display_name", name),
        ("description", description),
        ("visibility", visibility),
        ("accept_calls", accept_calls),
    ] {
        if let Some(v) = value {
            body[key] = json!(v);
        }
    }
    if body.as_object().is_some_and(|o| o.is_empty()) {
        anyhow::bail!(
            "nothing to change: pass --name, --description, --visibility or --accept-calls"
        );
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_body_sends_only_what_changes() {
        let v = update_body(None, None, None, Some("anyone".into())).unwrap();
        assert_eq!(v, json!({ "accept_calls": "anyone" }));
        let v = update_body(Some("Bot".into()), None, Some("network".into()), None).unwrap();
        assert_eq!(v, json!({ "display_name": "Bot", "visibility": "network" }));
        assert!(update_body(None, None, None, None).is_err());
    }
}
