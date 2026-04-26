//! `chakramcp` — command-line client for the relay.
//!
//! Two auth paths share one Bearer header at the wire:
//!   * `chakramcp login`               — OAuth 2.1 + PKCE
//!   * `chakramcp configure --api-key` — paste an `ck_…` API key
//!
//! Output is pretty-printed JSON to stdout so it pipes straight into jq
//! or your agent code. Human messages go to stderr.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;

mod auth;
mod client;
mod commands;
mod config;

use crate::client::ApiClient;
use crate::config::CliConfig;

#[derive(Parser, Debug)]
#[command(
    name = "chakramcp",
    version,
    about = "Command-line client for the ChakraMCP relay.",
    long_about = "Manage your agents, friendships, grants, and invocations from the terminal.\n\
                  Sign in with `chakramcp login` (OAuth) or `chakramcp configure --api-key …`."
)]
struct Cli {
    /// Override the app service URL (default: from config or http://localhost:8080).
    #[arg(long, env = "CHAKRAMCP_APP_URL", global = true)]
    app_url: Option<String>,

    /// Override the relay service URL (default: from config or http://localhost:8090).
    #[arg(long, env = "CHAKRAMCP_RELAY_URL", global = true)]
    relay_url: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Sign in via OAuth in the browser.
    Login,
    /// Configure an API key (alternative to `login`).
    Configure(commands::configure::Args),
    /// Forget any saved credentials.
    Logout,
    /// Show the currently signed-in user.
    Whoami,

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
        eprintln!("chakramcp: {err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = CliConfig::load()?;

    if let Some(u) = cli.app_url {
        cfg.server.app_url = u;
    }
    if let Some(u) = cli.relay_url {
        cfg.server.relay_url = u;
    }

    match cli.cmd {
        Cmd::Login => {
            auth::login(&mut cfg).await.context("login failed")?;
            // Confirm by calling /v1/me.
            let api = ApiClient::new(cfg.clone())?;
            let me: serde_json::Value = api.get_app("/v1/me").await?;
            eprintln!(
                "Signed in as {}.",
                me.pointer("/user/email").and_then(|v| v.as_str()).unwrap_or("?")
            );
        }
        Cmd::Configure(args) => commands::configure::run(args, &mut cfg).await?,
        Cmd::Logout => {
            cfg.auth = config::AuthConfig::default();
            cfg.save()?;
            eprintln!("Logged out.");
        }
        Cmd::Whoami => {
            let api = ApiClient::new(cfg)?;
            let me: serde_json::Value = api.get_app("/v1/me").await?;
            print(&json!({
                "auth": api.config().auth_kind(),
                "user": me.get("user"),
                "memberships": me.get("memberships"),
            }))?;
        }
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
            std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?
        };
        Ok(serde_json::from_str(&raw)?)
    } else {
        Ok(serde_json::from_str(s)?)
    }
}
