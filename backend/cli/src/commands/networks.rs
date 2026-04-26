//! `chakramcp networks` — manage configured networks.

use anyhow::{anyhow, bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::config::{AuthConfig, CliConfig, Network};
use crate::{print, ui};

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List configured networks. The active one is marked with a star.
    List,
    /// Switch the active network.
    Use { name: String },
    /// Add a new network. Auth is set up later via `login` or `configure`.
    Add {
        name: String,
        #[arg(long)]
        app_url: String,
        #[arg(long)]
        relay_url: String,
    },
    /// Remove a network and its stored credentials. Active flips to the next one if any.
    Remove { name: String },
    /// Show the active network's settings (URLs + auth kind).
    Show,
}

pub fn run(cmd: Cmd, cfg: &mut CliConfig) -> Result<()> {
    match cmd {
        Cmd::List => {
            let active = cfg.active.clone();
            let rows: Vec<Value> = cfg
                .networks
                .iter()
                .map(|n| {
                    json!({
                        "name": n.name,
                        "active": active.as_deref() == Some(n.name.as_str()),
                        "app_url": n.app_url,
                        "relay_url": n.relay_url,
                        "auth": n.auth_kind(),
                    })
                })
                .collect();
            print(&rows)
        }
        Cmd::Use { name } => {
            if cfg.network(&name).is_none() {
                bail!("no network named '{name}'");
            }
            cfg.active = Some(name.clone());
            cfg.save()?;
            ui::ok(&format!("switched to '{name}'"));
            Ok(())
        }
        Cmd::Add { name, app_url, relay_url } => {
            cfg.add_network(Network {
                name: name.clone(),
                app_url,
                relay_url,
                oauth_client_id: None,
                auth: AuthConfig::default(),
            })?;
            if cfg.active.is_none() {
                cfg.active = Some(name.clone());
            }
            cfg.save()?;
            ui::ok(&format!(
                "added '{name}'. run `chakramcp login --network {name}` to sign in"
            ));
            Ok(())
        }
        Cmd::Remove { name } => {
            cfg.remove_network(&name)?;
            cfg.save()?;
            ui::ok(&format!("removed '{name}'"));
            Ok(())
        }
        Cmd::Show => {
            let net = cfg
                .active_network()
                .ok_or_else(|| anyhow!("no active network"))?;
            print(&json!({
                "name": net.name,
                "app_url": net.app_url,
                "relay_url": net.relay_url,
                "auth": net.auth_kind(),
                "oauth_client_id": net.oauth_client_id,
            }))
        }
    }
}
