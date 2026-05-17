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
use chrono::Utc;
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

    /// Reply to a `message_owner` invocation that's sitting in your
    /// inbox. Sugar over `inbox respond` that builds the
    /// `message_owner`-shaped output JSON and posts the result with
    /// `confirmed_by_human: true` set automatically — the CLI runs at
    /// a human terminal, which is the canonical "a human is doing
    /// this" boundary the HITL gate gates on (issue #69 PR 2).
    ///
    /// For statuses other than `replied`, the reply text becomes
    /// optional and is dropped from the payload. For `deferred`, this
    /// sugar omits `defer_until`; use `inbox respond` directly if you
    /// need full control over the output JSON.
    Reply(ReplyArgs),
}

#[derive(Parser, Debug)]
pub struct ReplyArgs {
    /// The invocation id you're answering. Find these via
    /// `chakramcp inbox pull --agent <my-agent>`.
    pub invocation: String,
    /// The owner's reply text. Required when `--status=replied`
    /// (the default); ignored when status is `acknowledged`,
    /// `ignored`, or `deferred`.
    #[arg(default_value = "")]
    pub reply: String,
    /// One of `replied` (default), `acknowledged`, `ignored`,
    /// `deferred`. Matches the `message_owner` output_schema.
    #[arg(long, default_value = "replied",
        value_parser = ["replied", "acknowledged", "ignored", "deferred"])]
    pub status: String,
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
        Some(Sub::Reply(r)) => reply_run(r, &api).await,
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

// ─── `message reply` — sugar over `inbox respond` ────────

/// Build the `message_owner` output_schema-shaped JSON for a reply.
/// Extracted so the wire shape is unit-testable without a relay.
///
/// Spec'd shape (from the `message_owner` template's output_schema):
///   { status, reply?, responded_at, defer_until? }
///
/// `reply` is included only when `status == "replied"` (matches the
/// template's "Present only when status='replied'" rule). `deferred`
/// would normally also include `defer_until`, but the v1 sugar omits
/// it — callers who need it should fall back to `inbox respond` with
/// a hand-rolled `--output` payload.
fn build_reply_output(status: &str, reply: &str, now_rfc3339: String) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("status".into(), Value::String(status.to_string()));
    if status == "replied" {
        out.insert("reply".into(), Value::String(reply.to_string()));
    }
    out.insert("responded_at".into(), Value::String(now_rfc3339));
    Value::Object(out)
}

async fn reply_run(r: ReplyArgs, api: &ApiClient) -> Result<()> {
    if r.status == "replied" && r.reply.trim().is_empty() {
        bail!(
            "reply text is required when --status=replied. \
             Pass a non-empty <reply>, or use --status=acknowledged|ignored|deferred."
        );
    }

    let output = build_reply_output(&r.status, &r.reply, Utc::now().to_rfc3339());

    // confirmed_by_human: true — the CLI runs at a human terminal, so
    // by definition we're satisfying the HITL gate. This is the
    // ergonomic counterpart to the relay-side check in
    // `handlers::invoke::report_result`.
    let body = json!({
        "status": "succeeded",
        "output": output,
        "confirmed_by_human": true,
    });
    let resp: Value = api
        .post_relay(&format!("/v1/invocations/{}/result", r.invocation), &body)
        .await?;
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

// ─── Tests ────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const FIXED_TS: &str = "2026-05-17T12:00:00+00:00";

    #[test]
    fn build_reply_output_replied_includes_reply_text() {
        let v = build_reply_output("replied", "sure, friday works", FIXED_TS.into());
        assert_eq!(v["status"], json!("replied"));
        assert_eq!(v["reply"], json!("sure, friday works"));
        assert_eq!(v["responded_at"], json!(FIXED_TS));
        assert!(v.get("defer_until").is_none(), "deferred-only field");
    }

    #[test]
    fn build_reply_output_acknowledged_drops_reply() {
        let v = build_reply_output("acknowledged", "ignored", FIXED_TS.into());
        assert_eq!(v["status"], json!("acknowledged"));
        assert!(
            v.get("reply").is_none(),
            "reply must not be present when status != replied; got {v}"
        );
        assert_eq!(v["responded_at"], json!(FIXED_TS));
    }

    #[test]
    fn build_reply_output_ignored_drops_reply() {
        let v = build_reply_output("ignored", "whatever", FIXED_TS.into());
        assert_eq!(v["status"], json!("ignored"));
        assert!(v.get("reply").is_none());
    }

    #[test]
    fn build_reply_output_deferred_drops_reply_and_omits_defer_until() {
        // The CLI sugar deliberately leaves `defer_until` off — for
        // full control the caller falls back to `inbox respond`.
        let v = build_reply_output("deferred", "later", FIXED_TS.into());
        assert_eq!(v["status"], json!("deferred"));
        assert!(v.get("reply").is_none());
        assert!(v.get("defer_until").is_none());
    }

    #[test]
    fn reply_args_parse_default_status_is_replied() {
        use clap::Parser;
        #[derive(Parser, Debug)]
        struct Wrap {
            #[command(subcommand)]
            sub: Sub,
        }
        let w = Wrap::try_parse_from(["x", "reply", "00000000-0000-0000-0000-000000000000", "hi"])
            .unwrap();
        match w.sub {
            Sub::Reply(r) => {
                assert_eq!(r.status, "replied");
                assert_eq!(r.reply, "hi");
                assert_eq!(r.invocation, "00000000-0000-0000-0000-000000000000");
            }
            _ => panic!("expected Reply"),
        }
    }

    #[test]
    fn reply_args_parse_acknowledged_without_reply_text() {
        // `reply` has a default_value of "", so leaving it off when
        // --status=acknowledged is valid.
        use clap::Parser;
        #[derive(Parser, Debug)]
        struct Wrap {
            #[command(subcommand)]
            sub: Sub,
        }
        let w = Wrap::try_parse_from([
            "x",
            "reply",
            "00000000-0000-0000-0000-000000000000",
            "--status",
            "acknowledged",
        ])
        .unwrap();
        match w.sub {
            Sub::Reply(r) => {
                assert_eq!(r.status, "acknowledged");
                assert_eq!(r.reply, "");
            }
            _ => panic!("expected Reply"),
        }
    }
}
