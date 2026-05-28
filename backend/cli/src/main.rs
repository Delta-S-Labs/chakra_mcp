//! `chakramcp` — command-line client for the relay.
//!
//! Three auth paths share one Bearer header at the wire:
//!
//! * `chakramcp login` — OAuth 2.1 + PKCE (same device).
//! * `chakramcp pair` — device-flow (RFC 8628; user and agent on
//!   different devices, QR or 8-char code).
//! * `chakramcp configure --api-key` — paste an `ck_…` API key.
//!
//! Output is pretty-printed JSON to stdout so it pipes straight into jq
//! or your agent code. Human messages go to stderr.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

mod auth;
mod client;
mod commands;
mod config;
mod onboarding;
mod templates;
mod ui;
mod waiters;

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

/// How `chakramcp login` should authenticate. Without `--method` and
/// on an interactive terminal, the CLI runs the picker. Specifying
/// `--method` skips the picker — required when running in an agent /
/// CI environment where stdin is not a TTY.
#[derive(ValueEnum, Clone, Debug)]
pub enum LoginMethod {
    /// OAuth 2.1 + PKCE in the local browser (same device). Falls
    /// back to printing the URL if the browser can't be opened.
    Browser,
    /// Paste / supply an `ck_…` API key. Combine with `--api-key`
    /// or the `CHAKRAMCP_API_KEY` env var to skip the prompt.
    ApiKey,
    /// Device-flow (RFC 8628) — agent and user on different devices.
    /// Equivalent to `chakramcp pair`. Useful when this terminal has
    /// no browser of its own.
    Device,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Sign in. Interactive picker by default; pass `--method` to skip
    /// the picker in headless / agent environments.
    Login {
        /// Sign in to a specific network (defaults to the active one or runs the wizard).
        #[arg(long)]
        network: Option<String>,

        /// Skip the picker and use a specific auth method. Required
        /// when stdin isn't a TTY (CI, agent runtimes, `sh -c …`).
        #[arg(long, value_enum)]
        method: Option<LoginMethod>,

        /// API key to use when `--method api-key` is selected. Also
        /// readable from `CHAKRAMCP_API_KEY`. Required for fully
        /// non-interactive api-key login.
        #[arg(long, env = "CHAKRAMCP_API_KEY")]
        api_key: Option<String>,
    },
    /// Device-flow pair (RFC 8628). Use when this agent and the user
    /// are on different devices — prints a QR URL, a clickable URL,
    /// and an 8-char code, then polls until the user approves on
    /// chakramcp.com/app/pair. No API-key paste, no in-terminal QR
    /// renderer needed.
    Pair(commands::pair::Args),

    /// Configure an API key (alternative to `login` / `pair`).
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

    /// View / manage organization accounts you belong to (incl. settings).
    #[command(subcommand, alias = "orgs")]
    Org(commands::orgs::Cmd),

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

    /// Search the public agent directory by query, capability, tags, mode.
    /// Wraps `/v1/discovery/agents` (D10a). Bearer is optional —
    /// the endpoint is public, but auth lets the relay audit who
    /// searched for what.
    Discover(commands::discover::DiscoverArgs),

    /// Manage capabilities (the typed RPC surfaces an agent publishes).
    /// `add` creates a new one; peers can call it via `chakramcp invoke`
    /// once they have a friendship + grant.
    #[command(subcommand, alias = "cap")]
    Capabilities(commands::capabilities::Cmd),

    /// Sugar: send a `message_owner` ping to a friend. Resolves the
    /// grant automatically — equivalent to `chakramcp invoke` against
    /// the reserved `message_owner` capability on the peer.
    Message(commands::message::Args),

    /// Read or write agent ratings & reviews (migration 0023).
    /// `list`, `write`, `eligibility`, `hide`, `unhide`.
    #[command(subcommand, alias = "review")]
    Reviews(commands::reviews::Cmd),
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        ui::err(&format!("{err:#}"));
        // PRD §"Deterministic exit codes": wait/ensure commands return
        // a WaitError wrapped in anyhow. Downcast so callers can branch
        // on `$?` instead of grepping stderr. Anything else keeps the
        // pre-existing "any error = 1" behaviour.
        let code = err
            .downcast_ref::<crate::waiters::WaitError>()
            .map(|w| w.exit_code())
            .unwrap_or(1);
        std::process::exit(code);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = CliConfig::load()?;

    if let Some(name) = cli.network.clone() {
        if cfg.network(&name).is_none() {
            anyhow::bail!("no network named '{name}' — see `chakramcp networks list`");
        }
        cfg.active = Some(name);
    }

    match cli.cmd {
        Cmd::Login {
            network,
            method,
            api_key,
        } => {
            // `--method device` is a thin alias for `chakramcp pair` so
            // headless users discover one entry point. We dispatch to
            // pair directly rather than duplicating the polling loop.
            if matches!(method, Some(LoginMethod::Device)) {
                if let Some(name) = network {
                    if cfg.network(&name).is_none() {
                        anyhow::bail!("no network named '{name}'");
                    }
                    cfg.active = Some(name);
                }
                commands::pair::run(commands::pair::Args::default(), &mut cfg).await?;
                return Ok(());
            }
            let outcome = onboarding::run_login(
                &mut cfg,
                onboarding::LoginOptions {
                    network,
                    method: method.map(onboarding::Mode::from),
                    api_key,
                },
            )
            .await?;
            if let Some(email) = outcome.display_account {
                ui::closing(&outcome.network, &email);
            }
        }
        Cmd::Pair(args) => commands::pair::run(args, &mut cfg).await?,
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
        Cmd::Org(cmd) => commands::orgs::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Network => {
            let api = ApiClient::new(cfg)?;
            let agents: serde_json::Value = api.get_relay("/v1/network/agents").await?;
            print(&agents)?;
        }
        Cmd::Friendships(cmd) => commands::friendships::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Grants(cmd) => commands::grants::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Invoke(args) => commands::invoke::run(args, ApiClient::new(cfg)?).await?,
        Cmd::Inbox(cmd) => commands::inbox::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Discover(args) => commands::discover::run(args, ApiClient::new(cfg)?).await?,
        Cmd::Capabilities(cmd) => commands::capabilities::run(cmd, ApiClient::new(cfg)?).await?,
        Cmd::Message(args) => commands::message::run(args, ApiClient::new(cfg)?).await?,
        Cmd::Reviews(cmd) => commands::reviews::run(cmd, ApiClient::new(cfg)?).await?,
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
            std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {path}: {e}"))?
        };
        Ok(serde_json::from_str(&raw)?)
    } else {
        Ok(serde_json::from_str(s)?)
    }
}
