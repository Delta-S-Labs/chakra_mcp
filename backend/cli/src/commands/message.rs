//! `chakramcp message …` — two flavours:
//!
//! 1. `chakramcp message <peer> <text>` (fire-and-forget) — sugar over
//!    `invoke` for the reserved `message_owner` capability. Resolves
//!    the grant from *my inbound grants* and filters to
//!    capability_name=message_owner + granter matching the peer slug.
//!    Errors clearly when no matching grant exists. No `--wait` flag:
//!    `message_owner` is human-in-the-loop and may take hours to
//!    answer; the bare form is fire-and-forget by design.
//!
//! 2. `chakramcp message ensure <peer> <text> --from <my-agent> …` —
//!    thin alias over `invoke ensure` that supplies the
//!    `message_owner` capability and shapes the input. Use this when
//!    you want the CLI to propose the friendship + handle the deferred
//!    states. Issue #68 M3.

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::commands::invoke;
use crate::print;

/// Top-level args. Subcommand is optional so the legacy positional
/// form `message <peer> <text>` keeps working.
#[derive(Parser, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    pub sub: Option<Sub>,

    #[command(flatten)]
    pub send: SendArgs,
}

#[derive(Parser, Debug, Default)]
pub struct SendArgs {
    /// Peer to ping. Either `account-slug/agent-slug` or just
    /// `agent-slug` (if there's no ambiguity in your grants).
    pub peer: Option<String>,
    /// The message body. Plain text. Max 4000 chars.
    pub text: Option<String>,
    /// `low` (batch in digest) | `normal` (default) | `high` (alert).
    #[arg(long, default_value = "normal")]
    pub urgency: String,
    /// If false, the message is informational — owner can ack without typing.
    #[arg(long)]
    pub no_reply_expected: bool,
    /// One of your agent slugs — required only if multiple of your
    /// agents have a grant to call this peer's `message_owner`.
    #[arg(long = "as")]
    pub as_agent: Option<String>,
    /// Override the visible "from" name on the peer's side. Defaults
    /// to your agent's display_name as the relay sees it.
    #[arg(long)]
    pub from_display_name: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// Discover + friend + grant-check + invoke `message_owner`.
    /// Returns `waiting_for_friendship` / `waiting_for_grant` JSON
    /// (exit 3) when prerequisites are missing — pass
    /// `--wait-for-friendship` / `--wait-for-grant` to block instead.
    /// This is a thin alias over `chakramcp invoke ensure` with the
    /// capability fixed to `message_owner`.
    Ensure(EnsureArgs),
}

/// Args mirror `invoke::EnsureArgs` minus the capability_name +
/// input_json (we build those from `<text>` here).
#[derive(Parser, Debug)]
pub struct EnsureArgs {
    /// Peer to message. `account-slug/agent-slug` or bare `agent-slug`.
    pub peer: String,
    /// Message body.
    pub text: String,
    /// Your agent slug — the grantee that will send the message.
    #[arg(long)]
    pub from: String,

    /// Poll until the message_owner invocation reaches a terminal state.
    #[arg(long)]
    pub wait: bool,

    /// Block on friendship acceptance (default: return immediately
    /// with `waiting_for_friendship` if not accepted).
    #[arg(long)]
    pub wait_for_friendship: bool,
    #[arg(long, default_value_t = 300)]
    pub friendship_timeout: u64,

    /// Block on grant arrival (default: return immediately with
    /// `waiting_for_grant` if missing). The peer's owner has to issue
    /// the grant — `ensure` never auto-issues.
    #[arg(long)]
    pub wait_for_grant: bool,
    #[arg(long, default_value_t = 300)]
    pub grant_timeout: u64,

    #[arg(long, default_value_t = 300)]
    pub invoke_timeout: u64,
    #[arg(long, default_value_t = 2)]
    pub poll_interval: u64,

    #[arg(long)]
    pub json: bool,

    /// `low` | `normal` (default) | `high`.
    #[arg(long, default_value = "normal")]
    pub urgency: String,
    /// Informational — owner can ack without typing.
    #[arg(long)]
    pub no_reply_expected: bool,
    /// Override "from" display name.
    #[arg(long)]
    pub from_display_name: Option<String>,
}

pub async fn run(args: Args, api: ApiClient) -> Result<()> {
    match args.sub {
        Some(Sub::Ensure(e)) => ensure_run(e, &api).await,
        None => legacy_send(args.send, &api).await,
    }
}

// ─── Legacy fire-and-forget form ─────────────────────────

async fn legacy_send(send: SendArgs, api: &ApiClient) -> Result<()> {
    let peer_str = send.peer.ok_or_else(|| {
        anyhow!("usage: chakramcp message <peer> <text>  (or `chakramcp message ensure …`)")
    })?;
    let text = send
        .text
        .ok_or_else(|| anyhow!("missing <text> argument"))?;

    // 1. Parse the peer slug. Accept both `acct/slug` and `slug`.
    let (peer_account, peer_agent): (Option<&str>, &str) = match peer_str.split_once('/') {
        Some((a, b)) => (Some(a), b),
        None => (None, peer_str.as_str()),
    };

    // 2. Find inbound grants for message_owner whose granter slug matches.
    //    Inbound = I'm the grantee = I can call the peer.
    let grants: Value = api
        .get_relay("/v1/grants?direction=inbound&status=active")
        .await?;
    let empty = vec![];
    let grants = grants.as_array().unwrap_or(&empty);

    let mut matches: Vec<&Value> = grants
        .iter()
        .filter(|g| {
            g.pointer("/capability_name")
                .and_then(|v| v.as_str())
                .map(|n| n == "message_owner")
                .unwrap_or(false)
                && g.pointer("/granter/slug")
                    .and_then(|v| v.as_str())
                    .map(|s| s == peer_agent)
                    .unwrap_or(false)
                && peer_account
                    .map(|wanted_acct| {
                        g.pointer("/granter/account_slug")
                            .and_then(|v| v.as_str())
                            .map(|s| s == wanted_acct)
                            .unwrap_or(false)
                    })
                    .unwrap_or(true)
        })
        .collect();

    // Optional `--as` filter restricts to grants where I'm a
    // specific one of my agents.
    if let Some(ref as_slug) = send.as_agent {
        matches.retain(|g| {
            g.pointer("/grantee/slug")
                .and_then(|v| v.as_str())
                .map(|s| s == as_slug)
                .unwrap_or(false)
        });
    }

    let grant = match matches.len() {
        0 => bail!(
            "no active `message_owner` grant from {peer} to you.\n\n\
             Either:\n\
             - The peer hasn't published `message_owner` yet (ask them to run `chakramcp capabilities add --template message_owner`),\n\
             - You aren't friended yet (try `chakramcp friendships propose --to {peer}`),\n\
             - Or the friendship is accepted but no grant has been issued yet (the peer's agent owner has to run `chakramcp grants create ...`).\n\n\
             Tip: `chakramcp message ensure {peer} \"{text}\" --from <my-agent>` will propose + emit `waiting_for_grant` for you.",
            peer = peer_str
        ),
        1 => matches[0],
        n => bail!(
            "{n} of your agents have a grant to call {peer}'s `message_owner`. \
             Disambiguate with `--as <my-agent-slug>`. Candidates: {}",
            matches
                .iter()
                .filter_map(|g| g.pointer("/grantee/slug").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(", "),
            n = n,
            peer = peer_str,
        ),
    };

    let grant_id = grant
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("grant row missing id"))?;
    let grantee_id = grant
        .pointer("/grantee/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("grant row missing grantee.id"))?;

    // 3. Build the message_owner input + invoke.
    let mut input = json!({
        "message": text,
        "urgency": send.urgency,
        "expects_reply": !send.no_reply_expected,
    });
    if let Some(name) = send.from_display_name {
        input["from_display_name"] = Value::String(name);
    }

    let body = json!({
        "grant_id": grant_id,
        "grantee_agent_id": grantee_id,
        "input": input,
    });
    let resp: Value = api.post_relay("/v1/invoke", &body).await?;
    print(&resp)
}

// ─── `message ensure` — thin alias over `invoke ensure` ──

async fn ensure_run(e: EnsureArgs, api: &ApiClient) -> Result<()> {
    // Build the message_owner payload. Note: the bare `message`
    // command adds urgency/expects_reply/from_display_name; we carry
    // those through here for parity.
    let mut input = json!({
        "message": e.text,
        "urgency": e.urgency,
        "expects_reply": !e.no_reply_expected,
    });
    if let Some(name) = e.from_display_name {
        input["from_display_name"] = Value::String(name);
    }
    let input_json = serde_json::to_string(&input)?;

    let inner = invoke::EnsureArgs {
        peer: e.peer,
        capability_name: "message_owner".to_string(),
        input_json,
        from: e.from,
        wait: e.wait,
        wait_for_friendship: e.wait_for_friendship,
        friendship_timeout: e.friendship_timeout,
        wait_for_grant: e.wait_for_grant,
        grant_timeout: e.grant_timeout,
        invoke_timeout: e.invoke_timeout,
        poll_interval: e.poll_interval,
        json: e.json,
    };
    invoke::ensure_run(inner, api).await
}
