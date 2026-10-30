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
        /// What it does, in a sentence. At least 20 characters for a
        /// `network` agent: it's what people browsing the directory read.
        #[arg(long, default_value = "")]
        description: String,
        /// `private` (default), `org` or `network`.
        #[arg(long, default_value = "private")]
        visibility: String,
        /// A directory tag (repeatable): 1-32 lowercase letters, digits or
        /// hyphens, at most 10. E.g. `--tag scheduling --tag travel`.
        #[arg(long = "tag")]
        tags: Vec<String>,
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
    /// Fetch one agent, yours or another's: its id, `account/agent`, or a slug.
    Get { id: String },
    /// An agent's profile: its owner, capabilities with their schemas,
    /// reviews, how to call it, and your standing with it. Takes its id,
    /// `account/agent`, or a slug.
    Show { agent: String },
    /// Change one of your agents (its id, `account/agent`, or a slug): its
    /// name, description, visibility, tags, or who may call it.
    Update {
        id: String,
        /// Display name.
        #[arg(long)]
        name: Option<String>,
        /// What it does, in a sentence (20+ characters for a `network` agent).
        #[arg(long)]
        description: Option<String>,
        /// `private`, `org` or `network`.
        #[arg(long)]
        visibility: Option<String>,
        /// Who may call this agent: its friends (the default), anyone, or nobody.
        #[arg(long, value_parser = ["friends", "anyone", "nobody"])]
        accept_calls: Option<String>,
        /// Replace its directory tags (repeatable).
        #[arg(long = "tag", conflicts_with = "clear_tags")]
        tags: Vec<String>,
        /// Remove all its tags.
        #[arg(long)]
        clear_tags: bool,
    },
    /// A pull agent's wake URL: a URL the relay pings when work arrives, at
    /// most every 30 seconds, so the agent needn't keep polling.
    #[command(subcommand)]
    Wake(WakeCmd),
}

#[derive(Subcommand, Debug)]
pub enum WakeCmd {
    /// Show the agent's wake URL and how its pings are doing.
    Show { agent: String },
    /// Set the agent's wake URL. The answer's `secret`, shown this once,
    /// verifies the `ChakraMCP-Signature` header on JSON pings.
    Set {
        agent: String,
        #[arg(long)]
        url: String,
        /// `json` (signed counts) or `text` (one line, e.g. for an ntfy topic).
        #[arg(long, default_value = "json", value_parser = ["json", "text"])]
        format: String,
    },
    /// Ping the wake URL now and show how it went.
    Test { agent: String },
    /// Remove the agent's wake URL.
    Clear { agent: String },
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
            tags,
            agent_card_url,
        } => {
            let mut body = json!({
                "account_id": account,
                "slug": slug,
                "display_name": name,
                "description": description,
                "visibility": visibility,
            });
            if !tags.is_empty() {
                body["tags"] = json!(tags);
            }
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
            let id = crate::resolve::peer_id(&api, &id).await?;
            let agent: Value = api.get_relay(&format!("/v1/agents/{id}")).await?;
            print(&agent)
        }
        Cmd::Show { agent } => {
            let path = crate::resolve::profile_path(&api, &agent).await?;
            let profile: Value = api.get_relay(&path).await?;
            print(&profile)
        }
        Cmd::Update {
            id,
            name,
            description,
            visibility,
            accept_calls,
            tags,
            clear_tags,
        } => {
            let tags = (clear_tags || !tags.is_empty()).then_some(tags);
            let body = update_body(name, description, visibility, accept_calls, tags)?;
            let id = crate::resolve::mine_id(&api, &id).await?;
            let agent: Value = api.patch_relay(&format!("/v1/agents/{id}"), &body).await?;
            print(&agent)
        }
        Cmd::Wake(cmd) => wake(cmd, &api).await,
    }
}

async fn wake(cmd: WakeCmd, api: &ApiClient) -> Result<()> {
    match cmd {
        WakeCmd::Show { agent } => {
            let id = crate::resolve::mine_id(api, &agent).await?;
            let v: Value = api.get_relay(&format!("/v1/agents/{id}/wake")).await?;
            print(&v)
        }
        WakeCmd::Set { agent, url, format } => {
            let id = crate::resolve::mine_id(api, &agent).await?;
            let v: Value = api
                .put_relay(
                    &format!("/v1/agents/{id}/wake"),
                    &json!({ "url": url, "format": format }),
                )
                .await?;
            eprintln!("Keep the secret: it's shown only now, and verifies the pings' signature.");
            print(&v)
        }
        WakeCmd::Test { agent } => {
            let id = crate::resolve::mine_id(api, &agent).await?;
            let v: Value = api
                .post_relay(&format!("/v1/agents/{id}/wake/test"), &json!({}))
                .await?;
            print(&v)
        }
        WakeCmd::Clear { agent } => {
            let id = crate::resolve::mine_id(api, &agent).await?;
            let _: Value = api.delete_relay(&format!("/v1/agents/{id}/wake")).await?;
            eprintln!("Wake URL cleared.");
            Ok(())
        }
    }
}

fn update_body(
    name: Option<String>,
    description: Option<String>,
    visibility: Option<String>,
    accept_calls: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<Value> {
    let mut body = json!({});
    if let Some(tags) = tags {
        body["tags"] = json!(tags);
    }
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
            "nothing to change: pass --name, --description, --visibility, --tag, --clear-tags or --accept-calls"
        );
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_body_sends_only_what_changes() {
        let v = update_body(None, None, None, Some("anyone".into()), None).unwrap();
        assert_eq!(v, json!({ "accept_calls": "anyone" }));
        let v = update_body(Some("Bot".into()), None, Some("network".into()), None, None).unwrap();
        assert_eq!(v, json!({ "display_name": "Bot", "visibility": "network" }));
        assert!(update_body(None, None, None, None, None).is_err());
    }

    #[test]
    fn wake_commands_parse() {
        use clap::Parser;
        for line in [
            "chakramcp agents wake show bob/venue",
            "chakramcp agents wake set bob/venue --url https://ntfy.sh/venue --format text",
            "chakramcp agents wake test bob/venue",
            "chakramcp agents wake clear bob/venue",
        ] {
            crate::Cli::try_parse_from(line.split(' ')).unwrap_or_else(|e| panic!("{line}: {e}"));
        }
        assert!(crate::Cli::try_parse_from(
            "chakramcp agents wake set bob/venue --url x --format xml".split(' ')
        )
        .is_err());
    }

    #[test]
    fn update_body_replaces_or_clears_tags() {
        let v = update_body(None, None, None, None, Some(vec!["travel".into()])).unwrap();
        assert_eq!(v, json!({ "tags": ["travel"] }));
        let v = update_body(None, None, None, None, Some(vec![])).unwrap();
        assert_eq!(v, json!({ "tags": [] }), "--clear-tags sends an empty list");
    }
}
