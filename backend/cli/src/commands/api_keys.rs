//! `chakramcp api-keys …`: create, list and revoke your API keys (`ck_…`),
//! which the SDKs, and MCP clients that take a key, sign in with. On a
//! self-hosted server with no dashboard, this is how you get one.
//!
//!   * `api-keys create --name ci`  — the key is printed once.
//!   * `api-keys list`              — your keys, never their secrets.
//!   * `api-keys revoke <id>`

use anyhow::{anyhow, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Create a key. It's shown once: store it somewhere safe.
    Create {
        /// What the key is for, e.g. `ci` or `laptop`.
        #[arg(long)]
        name: String,
        /// Limit the key to one account: a slug from `chakramcp org list`.
        #[arg(long)]
        account: Option<String>,
        /// Expire the key after this many days. Default: never.
        #[arg(long)]
        expires_in_days: Option<i64>,
        /// Which agents the key may manage: `all` (the default) or `own`,
        /// only agents created with it.
        #[arg(long, value_parser = ["all", "own"])]
        agent_scope: Option<String>,
    },
    /// List your keys. Secrets are never shown again.
    List,
    /// Revoke a key, by the id `list` shows.
    Revoke { id: String },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::Create {
            name,
            account,
            expires_in_days,
            agent_scope,
        } => {
            let account_id = match account {
                Some(slug) => Some(account_id_for(&api, &slug).await?),
                None => None,
            };
            let body = create_body(&name, account_id, expires_in_days, agent_scope);
            let created: Value = api.post_app("/v1/api-keys", &body).await?;
            print(&created)
        }
        Cmd::List => {
            let keys: Value = api.get_app("/v1/api-keys").await?;
            print(&keys)
        }
        Cmd::Revoke { id } => {
            let _: Value = api.delete_app(&format!("/v1/api-keys/{id}")).await?;
            print(&json!({ "revoked": id }))
        }
    }
}

/// The endpoint takes an account id; people know their accounts by slug.
async fn account_id_for(api: &ApiClient, slug: &str) -> Result<String> {
    let me: Value = api.get_app("/v1/me").await?;
    me["memberships"]
        .as_array()
        .and_then(|memberships| memberships.iter().find(|m| m["slug"] == slug))
        .and_then(|m| m["account_id"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow!("you're not a member of an account called {slug:?} (see `chakramcp org list`)")
        })
}

fn create_body(
    name: &str,
    account_id: Option<String>,
    expires_in_days: Option<i64>,
    agent_scope: Option<String>,
) -> Value {
    let mut body = json!({ "name": name });
    if let Some(id) = account_id {
        body["account_id"] = json!(id);
    }
    if let Some(days) = expires_in_days {
        body["expires_in_days"] = json!(days);
    }
    if let Some(scope) = agent_scope {
        body["agent_scope"] = json!(scope);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_carries_only_what_was_given() {
        assert_eq!(create_body("ci", None, None, None), json!({ "name": "ci" }));
        assert_eq!(
            create_body(
                "ci",
                Some("0199-account".into()),
                Some(30),
                Some("own".into())
            ),
            json!({
                "name": "ci",
                "account_id": "0199-account",
                "expires_in_days": 30,
                "agent_scope": "own",
            })
        );
    }
}
