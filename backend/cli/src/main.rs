//! `chakramcp` — command-line client for the relay.
//!
//! Two auth paths share one Bearer header at the wire:
//!   * `chakramcp login`               — OAuth 2.1 + PKCE
//!   * `chakramcp configure --api-key` — paste an `ck_…` API key
//!
//! Output is pretty-printed JSON to stdout so it pipes straight into jq
//! or your agent code. Human messages go to stderr.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod config;
mod onboarding;
mod ui;

use crate::client::ApiClient;
use crate::config::{AuthConfig, CliConfig};

#[derive(Parser, Debug)]
#[command(
    name = "chakramcp",
    version,
    about = "Command-line client for the ChakraMCP relay.",
    long_about = "Manage your agents, friendships, grants, and invocations from the terminal.\n\
                  Sign in with `chakramcp login` (OAuth) or `chakramcp configure --api-key …`."
)]
struct Cli {
    /// Override the network to use for this command (defaults to the active one).
    #[arg(long, env = "CHAKRAMCP_NETWORK", global = true)]
    network: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Sign in via OAuth in the browser.
    Login {
        /// Sign in to a specific network (defaults to the active one or runs the wizard).
        #[arg(long)]
        network: Option<String>,
    },
    /// Configure an API key (alternative to `login`).
    Configure(commands::configure::Args),
    /// Forget any saved credentials for the active network.
    Logout,
    /// Show the currently signed-in user.
    Whoami,

    /// Manage configured networks (public, self-hosted, local dev).
    #[command(subcommand)]
    Networks(commands::networks::Cmd),

    /// Manage the agents you own.
    #[command(subcommand)]
    Agents(commands::agents::Cmd),

    /// List network-visible agents you can address.
    Network,

    /// Manage friendships (the social tie that gates grants).
    #[command(subcommand)]
    Friendships(commands::friendships::Cmd),

    /// Manage grants (specific capability access on top of friendships).
    #[command(subcommand)]
    Grants(commands::grants::Cmd),

    /// Enqueue an invocation against a granted capability.
    Invoke(commands::invoke::Args),

    /// Granter-side inbox — pull pending work and post results.
    #[command(subcommand)]
    Inbox(commands::inbox::Cmd),
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        ui::err(&format!("{err:#}"));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = CliConfig::load()?;

    if let Some(name) = cli.network.clone() {
        if cfg.network(&name).is_none() {
            anyhow::bail!(
                "no network named '{name}' — see `chakramcp networks list`"
            );
        }
        cfg.active = Some(name);
    }

    match cli.cmd {
        Cmd::Login { network } => {
            let outcome = onboarding::run_login(&mut cfg, network).await?;
            if let Some(email) = outcome.display_account {
                ui::closing(&outcome.network, &email);
            }
        }
        Cmd::Configure(args) => commands::configure::run(args, &mut cfg).await?,
        Cmd::Logout => {
            if let Some(net) = cfg.active_network_mut() {
                let name = net.name.clone();
                net.auth = AuthConfig::default();
                cfg.save()?;
                ui::ok(&format!("logged out of '{name}'"));
            } else {
                ui::note("no active network — nothing to do");
            }
        }
        Cmd::Whoami => {
            let api = ApiClient::new(cfg)?;
            let net = api.network()?;
            let me: serde_json::Value = api.get_app("/v1/me").await?;
            print(&serde_json::json!({
                "network": net.name,
                "auth": net.auth_kind(),
                "user": me.get("user"),
                "memberships": me.get("memberships"),
            }))?;
        }
        Cmd::Networks(cmd) => commands::networks::run(cmd, &mut cfg)?,
        Cmd::Agents(cmd) => commands::agents::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Network => {
            let api = ApiClient::new(cfg)?;
            let agents: serde_json::Value = api.get_relay("/v1/network/agents").await?;
            print(&agents)?;
        }
        Cmd::Friendships(cmd) => commands::friendships::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Grants(cmd) => commands::grants::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Invoke(args) => commands::invoke::run(args, ApiClient::new(cfg)?).await?,
        Cmd::Inbox(cmd) => commands::inbox::run(cmd, ApiClient::new(cfg)?).await?,
    }

    Ok(())
}

pub fn print<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Read a JSON arg. If it starts with '@' the rest is a file path.
pub fn read_json_arg(s: &str) -> Result<serde_json::Value> {
    if let Some(path) = s.strip_prefix('@') {
        let raw = if path == "-" {
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
            buf
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("reading {path}: {e}"))?
        };
        Ok(serde_json::from_str(&raw)?)
    } else {
        Ok(serde_json::from_str(s)?)
    }
}
