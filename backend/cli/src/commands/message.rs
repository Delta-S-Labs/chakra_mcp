//! `chakramcp message <peer> "<text>"` — sugar over `invoke` for the
//! reserved `message_owner` capability.
//!
//! Resolves the grant by looking at *my inbound grants* and filtering
//! to capability_name=message_owner + granter matching the peer slug.
//! Errors clearly when no matching grant exists ("you need to friend +
//! get granted first"). When multiple grants match (e.g. several of
//! my agents can call the same peer), require `--as <my-slug>` to
//! disambiguate.
//!
//! No `--wait` flag — message_owner has a human-in-the-loop handler
//! that may take minutes or hours to respond. The default behaviour
//! is fire-and-forget; the caller polls `chakramcp invocations get
//! <id>` (or just inspects the relay's audit log) for the eventual
//! reply.

use anyhow::{anyhow, bail, Result};
use clap::Parser;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

#[derive(Parser, Debug)]
pub struct Args {
    /// Peer to ping. Either `account-slug/agent-slug` or just
    /// `agent-slug` (if there's no ambiguity in your grants).
    pub peer: String,
    /// The message body. Plain text. Max 4000 chars.
    pub text: String,
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

pub async fn run(args: Args, api: ApiClient) -> Result<()> {
    // 1. Parse the peer slug. Accept both `acct/slug` and `slug`.
    let (peer_account, peer_agent): (Option<&str>, &str) = match args.peer.split_once('/') {
        Some((a, b)) => (Some(a), b),
        None => (None, args.peer.as_str()),
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
    if let Some(ref as_slug) = args.as_agent {
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
             - Or the friendship is accepted but no grant has been issued yet (the peer's agent owner has to run `chakramcp grants create ...`).",
            peer = args.peer
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
            peer = args.peer,
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
        "message": args.text,
        "urgency": args.urgency,
        "expects_reply": !args.no_reply_expected,
    });
    if let Some(name) = args.from_display_name {
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
