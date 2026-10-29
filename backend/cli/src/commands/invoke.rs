//! `chakramcp invoke …` — three flavours:
//!
//! 1. `chakramcp invoke --grant <id> --as <agent> --input <json> [--wait]`
//!    Original fire-and-forget enqueue. Kept verbatim for v0.1.x SDK
//!    compatibility (the published Python/TS SDKs shell out to this).
//!
//! 2. `chakramcp invoke wait <invocation_id> [--timeout 300]
//!     [--poll-interval 2] [--json]`
//!    Poll an existing invocation to terminal. Issue #68 M1.
//!
//! 3. `chakramcp invoke ensure <peer> <capability> <input>
//!     --from <my-agent> [--wait] [--wait-for-friendship]
//!     [--wait-for-grant] …`
//!    End-to-end orchestration: discover peer → ensure friendship →
//!    ensure grant → invoke → optionally wait. Issue #68 M3. Returns
//!    distinct `waiting_for_friendship` / `waiting_for_grant` states
//!    (exit code 3) when prerequisites are missing and the relevant
//!    `--wait-for-*` flag was not passed.
//!
//! We intentionally do NOT auto-issue grants. Grant issuance is the
//! consent boundary — the peer's owner must run
//! `chakramcp grants create` themselves.

use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use tokio::time::Instant;
use uuid::Uuid;

use crate::client::ApiClient;
use crate::waiters::{
    self, wait_for_friendship, wait_for_invocation, WaitError, MIN_POLL_INTERVAL,
};
use crate::{print, read_json_arg};

/// Top-level args for `chakramcp invoke …`.
///
/// `args_conflicts_with_subcommands` lets clap dispatch to either:
///   * the legacy positional flag form (no subcommand), or
///   * a subcommand (`wait`, `ensure`).
#[derive(Parser, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    pub sub: Option<Sub>,

    #[command(flatten)]
    pub send: SendArgs,
}

/// Legacy "submit an invocation now" args. Exactly one of `--grant`
/// (trusted path) or `--capability` (public-invoke path, migration
/// 0022) is required.
#[derive(Parser, Debug, Default)]
pub struct SendArgs {
    /// Grant id authorising this call. Trusted path: friendship +
    /// grant already in place. Mutually exclusive with `--capability`.
    #[arg(long, conflicts_with = "capability")]
    pub grant: Option<String>,
    /// Public-invoke path: target a capability directly by id without
    /// a friendship/grant. Only works for capabilities whose owner has
    /// set `public_invoke=true` (see `capabilities add --public-invoke
    /// --monthly-quota N`). Mutually exclusive with `--grant`.
    #[arg(long)]
    pub capability: Option<String>,
    /// Your agent that's making the call (the grantee side).
    #[arg(long = "as")]
    pub as_agent: Option<String>,
    /// Input — JSON literal, or `@path` to read from a file, or `@-` for stdin.
    #[arg(long, default_value = "{}")]
    pub input: String,
    /// Poll until the invocation reaches a terminal status, then print the row.
    #[arg(long)]
    pub wait: bool,
    /// For a capability that asks its owner first: how long you're asking
    /// to be allowed to call it without asking again — `once` (the
    /// default), `<n>h` or `<n>d`. The owner may approve less.
    #[arg(long)]
    pub consent: Option<String>,
    /// With `--consent`: the most calls the approval should cover.
    #[arg(long)]
    pub consent_max: Option<i32>,
    /// Why you're asking, shown to the owner when they decide.
    #[arg(long)]
    pub purpose: Option<String>,
    /// How long the call may wait to be answered before it times out:
    /// seconds, or `<n>m`, `<n>h`, `<n>d`. At most the server's default
    /// (24h; 7d for human_in_loop capabilities).
    #[arg(long, value_parser = parse_expires_in, value_name = "DURATION")]
    pub expires_in: Option<i64>,
}

/// `90`, `90s`, `30m`, `2h` or `1d` → seconds.
fn parse_expires_in(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    let bad = || format!("expected seconds or <n>s, <n>m, <n>h, <n>d, not {raw:?}");
    let (num, unit) = match raw.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&raw[..i], c),
        _ => (raw, 's'),
    };
    let n: i64 = num.parse().map_err(|_| bad())?;
    let per = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return Err(bad()),
    };
    n.checked_mul(per).filter(|s| *s > 0).ok_or_else(bad)
}

impl SendArgs {
    /// `consent_terms` for the invoke body, if any consent flag was given.
    fn consent_terms(&self) -> Option<Value> {
        if self.consent.is_none() && self.consent_max.is_none() && self.purpose.is_none() {
            return None;
        }
        let mut terms = json!({});
        if let Some(valid_for) = &self.consent {
            terms["valid_for"] = json!(valid_for);
        }
        if let Some(max) = self.consent_max {
            terms["max_requests"] = json!(max);
        }
        if let Some(purpose) = &self.purpose {
            terms["purpose"] = json!(purpose);
        }
        Some(terms)
    }
}

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// Poll an existing invocation until it reaches a terminal status.
    /// Terminal states: `succeeded | failed | rejected | timeout |
    /// cancelled`. Exit codes:
    ///   * `0` succeeded
    ///   * `2` deadline reached
    ///   * `3` failed / rejected / cancelled
    ///   * `4` auth/permission
    ///   * `5` transient
    ///   * `6` invalid arguments
    Wait(WaitArgs),
    /// Call a peer's capability by name. A public capability is called
    /// directly; otherwise ensure the friendship and grant first. Returns
    /// distinct `waiting_for_friendship` / `waiting_for_grant` JSON states
    /// (exit 3) when those are missing — pass `--wait-for-friendship` /
    /// `--wait-for-grant` to block instead.
    Ensure(EnsureArgs),
}

#[derive(Parser, Debug)]
pub struct WaitArgs {
    /// Invocation UUID to watch.
    pub invocation_id: String,
    /// Seconds before we give up (default 300).
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
    /// Seconds between polls (default 2, minimum 1).
    #[arg(long, default_value_t = 2)]
    pub poll_interval: u64,
    /// Emit a single JSON envelope on stdout (otherwise prints the
    /// full invocation row).
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug)]
pub struct EnsureArgs {
    /// Peer to invoke: its id, `account-slug/agent-slug`, or a bare
    /// `agent-slug` (one of your agents first, else unique on the network).
    pub peer: String,
    /// Name of the capability to invoke (e.g. `message_owner`).
    pub capability_name: String,
    /// Input — JSON literal, or `@path` to read from a file, or `@-` for stdin.
    pub input_json: String,
    /// Your agent (id, slug, or `account/agent`): the side that makes the call.
    #[arg(long)]
    pub from: String,

    /// After enqueueing, poll until the invocation reaches a terminal
    /// status. Mirrors `chakramcp invoke wait`.
    #[arg(long)]
    pub wait: bool,

    /// If no friendship exists, block until accepted (instead of
    /// returning `waiting_for_friendship` immediately).
    #[arg(long)]
    pub wait_for_friendship: bool,
    /// Friendship deadline in seconds when `--wait-for-friendship` is
    /// set.
    #[arg(long, default_value_t = 300)]
    pub friendship_timeout: u64,

    /// If no grant exists, block until one appears (instead of
    /// returning `waiting_for_grant` immediately). The peer's owner
    /// must run `chakramcp grants create` on their side — `ensure`
    /// never auto-issues grants.
    #[arg(long)]
    pub wait_for_grant: bool,
    /// Grant deadline in seconds when `--wait-for-grant` is set.
    #[arg(long, default_value_t = 300)]
    pub grant_timeout: u64,

    /// Invocation deadline in seconds when `--wait` is set.
    #[arg(long, default_value_t = 300)]
    pub invoke_timeout: u64,
    /// Seconds between polls for any of the waiters (minimum 1).
    #[arg(long, default_value_t = 2)]
    pub poll_interval: u64,

    /// Emit a single JSON envelope on stdout instead of the relay rows.
    #[arg(long)]
    pub json: bool,

    /// How long the call may wait to be answered before it times out:
    /// seconds, or `<n>m`, `<n>h`, `<n>d`.
    #[arg(long, value_parser = parse_expires_in, value_name = "DURATION")]
    pub expires_in: Option<i64>,
}

const LEGACY_POLL_INTERVAL_MS: u64 = 1500;
const LEGACY_POLL_MAX_ATTEMPTS: usize = 120; // 3 minutes — matches the previous bare `invoke --wait`.

pub async fn run(args: Args, api: ApiClient) -> Result<()> {
    match args.sub {
        Some(Sub::Wait(w)) => wait_run(w, &api).await,
        Some(Sub::Ensure(e)) => ensure_run(e, &api).await,
        None => legacy_send(args.send, &api).await,
    }
}

// ─── Legacy `invoke --grant … --as …` ─────────────────────

async fn legacy_send(send: SendArgs, api: &ApiClient) -> Result<()> {
    // Exactly one of --grant (trusted) or --capability (public-invoke).
    // Clap's `conflicts_with` on --grant already rejects both at the
    // arg layer; this is the "neither" guard with a useful message.
    let consent_terms = send.consent_terms();
    let as_agent = send
        .as_agent
        .ok_or_else(|| anyhow::anyhow!("--as is required"))?;
    let input = read_json_arg(&send.input)?;
    let mut body = match (send.grant, send.capability) {
        (Some(grant), None) => json!({
            "grant_id": grant,
            "grantee_agent_id": as_agent,
            "input": input,
        }),
        (None, Some(capability_id)) => json!({
            "capability_id": capability_id,
            "grantee_agent_id": as_agent,
            "input": input,
        }),
        (None, None) => {
            return Err(anyhow::anyhow!(
                "one of --grant (trusted) or --capability (public-invoke) is required — see `chakramcp grants list` or `chakramcp invoke ensure`"
            ));
        }
        (Some(_), Some(_)) => {
            // Should be unreachable via clap conflicts_with, but a
            // belt-and-braces guard keeps the error story honest.
            return Err(anyhow::anyhow!(
                "specify exactly one of --grant or --capability, not both"
            ));
        }
    };
    if let Some(terms) = consent_terms {
        body["consent_terms"] = terms;
    }
    if let Some(secs) = send.expires_in {
        body["expires_in"] = json!(secs);
    }
    let resp: Value = api.post_relay("/v1/invoke", &body).await?;
    if resp.get("status").and_then(Value::as_str) == Some("awaiting_consent") {
        eprintln!(
            "Waiting for the owner's approval (request {}). Their agent gets the link to approve it.",
            resp.get("consent_request_id")
                .and_then(Value::as_str)
                .unwrap_or("?")
        );
    }

    if !send.wait {
        return print(&resp);
    }

    let id = resp
        .get("invocation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("invoke response missing invocation_id"))?
        .to_string();

    eprintln!("Enqueued {id}. Waiting for terminal status…");
    let hinted = std::sync::atomic::AtomicBool::new(false);
    for attempt in 0..LEGACY_POLL_MAX_ATTEMPTS {
        tokio::time::sleep(Duration::from_millis(LEGACY_POLL_INTERVAL_MS)).await;
        let row: Value = api.get_relay(&format!("/v1/invocations/{id}")).await?;
        waiters::say_peer_hint(&row, &hinted);
        let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(status, "succeeded" | "failed" | "rejected" | "timeout") {
            return print(&row);
        }
        if attempt > 0 && attempt % 10 == 0 {
            eprintln!(
                "  still {status} after {}s…",
                (attempt + 1) * (LEGACY_POLL_INTERVAL_MS as usize) / 1000
            );
        }
    }
    anyhow::bail!("timed out after 3 minutes — re-run `chakramcp invoke wait <id>` or check `chakramcp inbox` on the granter side");
}

// ─── `invoke wait` ────────────────────────────────────────

async fn wait_run(w: WaitArgs, api: &ApiClient) -> Result<()> {
    let id = Uuid::parse_str(&w.invocation_id).map_err(|_| {
        anyhow::anyhow!(WaitError::InvalidArgs(format!(
            "not a UUID: {}",
            w.invocation_id
        )))
    })?;
    let timeout = Duration::from_secs(w.timeout.max(1)).min(Duration::from_secs(86_400));
    let poll = Duration::from_secs(w.poll_interval).max(MIN_POLL_INTERVAL);
    let deadline = Instant::now() + timeout;

    eprintln!(
        "Waiting on invocation {id} (timeout {}s, poll {}s)…",
        timeout.as_secs(),
        poll.as_secs()
    );

    match wait_for_invocation(api, id, deadline, poll).await {
        Ok(outcome) => {
            if w.json {
                print(&invocation_outcome_json(&outcome))?;
            } else {
                print(&outcome.row)?;
            }
            if outcome.status != "succeeded" {
                return Err(anyhow::anyhow!(WaitError::TerminalNo(outcome.status)));
            }
            Ok(())
        }
        Err(WaitError::Timeout {
            elapsed_ms,
            last_status,
        }) => {
            if w.json {
                print(&json!({
                    "ok": false,
                    "invocation_id": id,
                    "status": "timeout",
                    "elapsed_ms": elapsed_ms,
                    "last_observed_status": last_status,
                }))?;
            }
            Err(anyhow::anyhow!(WaitError::Timeout {
                elapsed_ms,
                last_status
            }))
        }
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

/// PRD §"C) `invoke wait`" — succeeded / failed envelopes.
pub(crate) fn invocation_outcome_json(out: &waiters::InvocationOutcome) -> Value {
    let ok = out.status == "succeeded";
    let capability_name = waiters::pointer_str(&out.row, "/capability_name");
    let output = out.row.pointer("/output_preview").cloned();
    let error = waiters::pointer_str(&out.row, "/error_message");

    if ok {
        json!({
            "ok": true,
            "invocation_id": out.invocation_id,
            "status": out.status,
            "capability_name": capability_name,
            "output": output,
            "elapsed_ms": out.elapsed_ms,
        })
    } else {
        json!({
            "ok": false,
            "invocation_id": out.invocation_id,
            "status": out.status,
            "capability_name": capability_name,
            "error": error,
            "elapsed_ms": out.elapsed_ms,
        })
    }
}

// ─── `invoke ensure` ──────────────────────────────────────

/// Parsed `<peer>` string. `agent_slug` is always present; `account_slug`
/// is `None` when the user passed just the bare slug.
#[derive(Debug, Clone)]
pub(crate) struct PeerRef {
    pub account_slug: Option<String>,
    pub agent_slug: String,
}

impl PeerRef {
    pub(crate) fn parse(s: &str) -> Result<Self, WaitError> {
        if s.is_empty() {
            return Err(WaitError::InvalidArgs("peer must not be empty".into()));
        }
        Ok(match s.split_once('/') {
            Some((acc, agent)) if !acc.is_empty() && !agent.is_empty() => PeerRef {
                account_slug: Some(acc.to_string()),
                agent_slug: agent.to_string(),
            },
            Some(_) => {
                return Err(WaitError::InvalidArgs(format!(
                    "peer slug malformed (expected `account/agent`): {s}"
                )))
            }
            None => PeerRef {
                account_slug: None,
                agent_slug: s.to_string(),
            },
        })
    }

    pub(crate) fn display(&self) -> String {
        match &self.account_slug {
            Some(acc) => format!("{acc}/{}", self.agent_slug),
            None => self.agent_slug.clone(),
        }
    }
}

/// Friendship-discovery result.
struct FriendshipState {
    /// Existing friendship row (if any).
    row: Option<Value>,
}

/// Find any friendship row between `my_agent_id` ↔ `peer_agent_id`,
/// regardless of direction or status. We want to know whether to
/// propose, wait, or treat as already-accepted.
async fn find_friendship(
    api: &ApiClient,
    my_agent_id: &str,
    peer_agent_id: &str,
) -> Result<FriendshipState, WaitError> {
    // The list endpoint returns rows for both directions when
    // direction=all; filter client-side. No `agent_id` query param
    // exists today, so we fetch all and filter — acceptable since
    // a user's friendship list is small.
    let v: Value = api
        .get_relay("/v1/friendships?direction=all")
        .await
        .map_err(|e| classify_http_err(&e))?;
    let empty = vec![];
    let rows = v.as_array().unwrap_or(&empty);
    let row = rows
        .iter()
        .find(|f| {
            let p = f
                .pointer("/proposer/id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let t = f
                .pointer("/target/id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (p == my_agent_id && t == peer_agent_id) || (p == peer_agent_id && t == my_agent_id)
        })
        .cloned();
    Ok(FriendshipState { row })
}

/// Find an active grant from peer → me for `capability_name`. Returns
/// the grant row (or None).
async fn find_grant(
    api: &ApiClient,
    my_agent_id: &str,
    peer_agent_id: &str,
    capability_name: &str,
) -> Result<Option<Value>, WaitError> {
    // No server-side capability filter on `/v1/grants` — filter here.
    // direction=inbound = I'm the grantee.
    let v: Value = api
        .get_relay("/v1/grants?direction=inbound&status=active")
        .await
        .map_err(|e| classify_http_err(&e))?;
    let empty = vec![];
    let rows = v.as_array().unwrap_or(&empty);
    Ok(rows
        .iter()
        .find(|g| {
            let cap = g
                .pointer("/capability_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let granter = g
                .pointer("/granter/id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let grantee = g
                .pointer("/grantee/id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            cap == capability_name && granter == peer_agent_id && grantee == my_agent_id
        })
        .cloned())
}

/// Map an `anyhow::Error` from `api.get_relay` / `post_relay` into our
/// `WaitError` taxonomy. The decode helper bails with the HTTP status
/// in the message; we sniff for it.
pub(crate) fn classify_http_err(e: &anyhow::Error) -> WaitError {
    let msg = format!("{e:#}");
    if msg.contains("401") {
        WaitError::Unauthorized(msg)
    } else if msg.contains("403") {
        WaitError::Forbidden(msg)
    } else if msg.contains("404") {
        WaitError::InvalidArgs(msg)
    } else {
        WaitError::Transient(msg)
    }
}

/// Entry point for `invoke ensure` — also reachable from `message ensure`.
pub(crate) async fn ensure_run(e: EnsureArgs, api: &ApiClient) -> Result<()> {
    let peer_ref = PeerRef::parse(&e.peer).map_err(anyhow::Error::from)?;
    let input = read_json_arg(&e.input_json)?;
    let poll = Duration::from_secs(e.poll_interval).max(MIN_POLL_INTERVAL);

    // 1. Resolve the peer + my agent. Both have to be UUIDs for
    //    downstream POSTs.
    eprintln!("Resolving peer `{}`…", peer_ref.display());
    let peer_row = crate::resolve::peer(api, &e.peer)
        .await
        .map_err(anyhow::Error::from)?;
    let peer_agent_id = peer_row
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("network agent row missing id"))?
        .to_string();
    let peer_account_slug = peer_row
        .get("account_slug")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let peer_agent_slug = peer_row
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let peer_display = format!("{peer_account_slug}/{peer_agent_slug}");

    let my_row = crate::resolve::mine(api, &e.from)
        .await
        .map_err(anyhow::Error::from)?;
    let my_agent_id = my_row
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("my agent row missing id"))?
        .to_string();

    // 1b. A public capability needs no friendship or grant: call it directly.
    // The peer's profile says which ones are public (relays from 0.6.0; an
    // older relay has no profiles, and the friendship-and-grant path runs).
    if let Some(capability_id) = public_capability(api, &peer_agent_id, &e.capability_name).await {
        let body = json!({
            "capability_id": capability_id,
            "grantee_agent_id": my_agent_id,
            "input": input,
        });
        let via = json!({ "access": "public", "capability": e.capability_name });
        return invoke_and_wait(api, &e, body, &peer_display, via, poll).await;
    }

    // 2. Friendship.
    let f = find_friendship(api, &my_agent_id, &peer_agent_id)
        .await
        .map_err(anyhow::Error::from)?;
    let friendship_row = match f.row {
        Some(row) => row,
        None => {
            // Propose. The user-as-proposer path goes through the
            // proposer's account-membership check on the relay.
            eprintln!("No friendship yet — proposing…");
            let body = json!({
                "proposer_agent_id": my_agent_id,
                "target_agent_id": peer_agent_id,
                "proposer_message": null,
            });
            let v: Value = api
                .post_relay("/v1/friendships", &body)
                .await
                .map_err(|err| anyhow::anyhow!(classify_http_err(&err)))?;
            v
        }
    };

    let friendship_id = friendship_row
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("friendship row missing id"))?
        .to_string();
    let mut friendship_status = friendship_row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if friendship_status != "accepted" {
        if e.wait_for_friendship {
            let fid = Uuid::parse_str(&friendship_id).map_err(|_| {
                anyhow::anyhow!(WaitError::InvalidArgs(format!(
                    "friendship id not a UUID: {friendship_id}"
                )))
            })?;
            let deadline = Instant::now() + Duration::from_secs(e.friendship_timeout.max(1));
            eprintln!(
                "Waiting for friendship {fid} (timeout {}s)…",
                e.friendship_timeout
            );
            let out = wait_for_friendship(api, fid, deadline, poll)
                .await
                .map_err(anyhow::Error::from)?;
            friendship_status = out.status.clone();
            if friendship_status != "accepted" {
                // Emit a terminal "no" envelope per PRD §E.
                if e.json {
                    print(&json!({
                        "ok": false,
                        "peer": peer_display,
                        "status": "friendship_not_accepted",
                        "friendship": {
                            "id": friendship_id,
                            "status": friendship_status,
                        },
                    }))?;
                }
                return Err(anyhow::anyhow!(WaitError::TerminalNo(friendship_status)));
            }
        } else {
            // Non-blocking branch — return the deferred-state envelope
            // with exit 3 per PRD §"Three locked design decisions".
            if e.json {
                print(&json!({
                    "ok": false,
                    "peer": peer_display,
                    "status": "waiting_for_friendship",
                    "friendship": {
                        "id": friendship_id,
                        "status": friendship_status,
                    },
                    "hint": "Peer must accept friendship. Re-run with --wait-for-friendship to block.",
                }))?;
            } else {
                print(&friendship_row)?;
            }
            return Err(anyhow::anyhow!(WaitError::TerminalNo(
                "waiting_for_friendship".into()
            )));
        }
    }

    // 3. Grant.
    let mut grant_row = find_grant(api, &my_agent_id, &peer_agent_id, &e.capability_name)
        .await
        .map_err(anyhow::Error::from)?;
    if grant_row.is_none() {
        if e.wait_for_grant {
            let deadline = Instant::now() + Duration::from_secs(e.grant_timeout.max(1));
            eprintln!(
                "Waiting for grant `{}` from {peer_display} (timeout {}s)…",
                e.capability_name, e.grant_timeout
            );
            loop {
                let g = find_grant(api, &my_agent_id, &peer_agent_id, &e.capability_name)
                    .await
                    .map_err(anyhow::Error::from)?;
                if let Some(found) = g {
                    grant_row = Some(found);
                    break;
                }
                let now = Instant::now();
                if now >= deadline {
                    if e.json {
                        print(&json!({
                            "ok": false,
                            "peer": peer_display,
                            "status": "timeout",
                            "waiting_for": "grant",
                            "required_capability": e.capability_name,
                        }))?;
                    }
                    return Err(anyhow::anyhow!(WaitError::Timeout {
                        elapsed_ms: 0,
                        last_status: Some("waiting_for_grant".into()),
                    }));
                }
                let remaining = deadline.saturating_duration_since(now);
                tokio::time::sleep(poll.min(remaining)).await;
            }
        } else {
            if e.json {
                print(&json!({
                    "ok": false,
                    "peer": peer_display,
                    "status": "waiting_for_grant",
                    "friendship": {
                        "id": friendship_id,
                        "status": "accepted",
                    },
                    "required_capability": e.capability_name,
                    "hint": "Peer owner must create grant. Re-run with --wait-for-grant to block.",
                }))?;
            }
            return Err(anyhow::anyhow!(WaitError::TerminalNo(
                "waiting_for_grant".into()
            )));
        }
    }
    let grant_row = grant_row.expect("grant present after waiting branch");
    let grant_id = grant_row
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("grant row missing id"))?
        .to_string();

    // 4. Invoke, and 5. optionally wait for the result.
    let body = json!({
        "grant_id": grant_id,
        "grantee_agent_id": my_agent_id,
        "input": input,
    });
    let via = json!({
        "friendship": {"status": "accepted", "id": friendship_id},
        "grant": {
            "capability": e.capability_name,
            "status": "active",
            "id": grant_id,
        },
    });
    invoke_and_wait(api, &e, body, &peer_display, via, poll).await
}

/// The id of `capability` on `peer` when anyone signed in may call it, from
/// the peer's profile. `None` when it needs a grant, isn't there, or the
/// profile can't be read (an older relay): the caller then takes the
/// friendship-and-grant path.
async fn public_capability(api: &ApiClient, peer: &str, capability: &str) -> Option<String> {
    let profile: Value = api
        .get_relay(&format!("/v1/discovery/agents/{peer}"))
        .await
        .ok()?;
    public_capability_in(&profile, capability)
}

fn public_capability_in(profile: &Value, capability: &str) -> Option<String> {
    profile["capabilities"]
        .as_array()?
        .iter()
        .find(|c| c["name"] == capability && c["access"] == "public")
        .and_then(|c| c["id"].as_str().map(str::to_owned))
}

/// `body`'s envelope fields plus `via`'s: how the call was allowed (a
/// friendship and grant, or public access).
fn with_via(mut envelope: Value, via: &Value) -> Value {
    if let (Some(out), Some(fields)) = (envelope.as_object_mut(), via.as_object()) {
        for (k, v) in fields {
            out.insert(k.clone(), v.clone());
        }
    }
    envelope
}

/// Invoke, then print the result, waiting for it with `--wait`.
async fn invoke_and_wait(
    api: &ApiClient,
    e: &EnsureArgs,
    mut body: Value,
    peer_display: &str,
    via: Value,
    poll: Duration,
) -> Result<()> {
    if let Some(secs) = e.expires_in {
        body["expires_in"] = json!(secs);
    }
    let invoke_resp: Value = api
        .post_relay("/v1/invoke", &body)
        .await
        .map_err(|err| anyhow::anyhow!(classify_http_err(&err)))?;
    let invocation_id = invoke_resp
        .get("invocation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("invoke response missing invocation_id"))?
        .to_string();

    if !e.wait {
        if e.json {
            print(&with_via(
                json!({
                    "ok": true,
                    "peer": peer_display,
                    "invocation": {
                        "id": invocation_id,
                        "status": invoke_resp.get("status"),
                    },
                }),
                &via,
            ))?;
        } else {
            print(&invoke_resp)?;
        }
        return Ok(());
    }

    let iid = Uuid::parse_str(&invocation_id).map_err(|_| {
        anyhow::anyhow!(WaitError::InvalidArgs(format!(
            "invocation id not a UUID: {invocation_id}"
        )))
    })?;
    let deadline = Instant::now() + Duration::from_secs(e.invoke_timeout.max(1));
    eprintln!(
        "Waiting for invocation {iid} (timeout {}s)…",
        e.invoke_timeout
    );
    let outcome = wait_for_invocation(api, iid, deadline, poll).await;

    match outcome {
        Ok(o) => {
            let succeeded = o.status == "succeeded";
            if e.json {
                let mut payload = with_via(
                    json!({
                        "ok": succeeded,
                        "peer": peer_display,
                        "invocation": {
                            "id": invocation_id,
                            "status": o.status,
                        },
                    }),
                    &via,
                );
                if succeeded {
                    payload["reply"] = o
                        .row
                        .pointer("/output_preview")
                        .cloned()
                        .unwrap_or(Value::Null);
                } else {
                    payload["error"] = o
                        .row
                        .pointer("/error_message")
                        .cloned()
                        .unwrap_or(Value::Null);
                }
                print(&payload)?;
            } else {
                print(&o.row)?;
            }
            if !succeeded {
                return Err(anyhow::anyhow!(WaitError::TerminalNo(o.status)));
            }
            Ok(())
        }
        Err(WaitError::Timeout {
            elapsed_ms,
            last_status,
        }) => {
            if e.json {
                print(&json!({
                    "ok": false,
                    "peer": peer_display,
                    "invocation": {"id": invocation_id, "status": "timeout"},
                    "elapsed_ms": elapsed_ms,
                    "last_observed_status": last_status,
                }))?;
            }
            Err(anyhow::anyhow!(WaitError::Timeout {
                elapsed_ms,
                last_status,
            }))
        }
        Err(other) => Err(anyhow::anyhow!(other)),
    }
}

// ─── Tests ────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_in_takes_seconds_or_units() {
        for (raw, secs) in [
            ("90", 90),
            ("90s", 90),
            ("30m", 1800),
            ("2h", 7200),
            ("1d", 86_400),
        ] {
            assert_eq!(parse_expires_in(raw), Ok(secs), "{raw}");
        }
        for raw in ["", "h", "2w", "-5", "0", "1.5h"] {
            assert!(parse_expires_in(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn consent_flags_become_consent_terms() {
        let args = Args::try_parse_from([
            "invoke",
            "--grant",
            "g",
            "--as",
            "a",
            "--consent",
            "7d",
            "--consent-max",
            "5",
            "--purpose",
            "weekly sync",
        ])
        .unwrap();
        assert_eq!(
            args.send.consent_terms(),
            Some(json!({"valid_for": "7d", "max_requests": 5, "purpose": "weekly sync"}))
        );
        let plain = Args::try_parse_from(["invoke", "--grant", "g", "--as", "a"]).unwrap();
        assert_eq!(plain.send.consent_terms(), None);
    }

    #[test]
    fn public_capabilities_come_from_the_profile() {
        let profile = json!({ "capabilities": [
            { "id": "c1", "name": "guide.ask", "access": "public" },
            { "id": "c2", "name": "hold", "access": "friends" },
        ]});
        assert_eq!(
            public_capability_in(&profile, "guide.ask").as_deref(),
            Some("c1")
        );
        assert_eq!(public_capability_in(&profile, "hold"), None);
        assert_eq!(public_capability_in(&profile, "missing"), None);
        assert_eq!(public_capability_in(&json!({}), "guide.ask"), None);
    }

    #[test]
    fn peer_ref_account_form() {
        let p = PeerRef::parse("acct-x/agent-y").unwrap();
        assert_eq!(p.account_slug.as_deref(), Some("acct-x"));
        assert_eq!(p.agent_slug, "agent-y");
        assert_eq!(p.display(), "acct-x/agent-y");
    }

    #[test]
    fn peer_ref_bare_form() {
        let p = PeerRef::parse("agent-y").unwrap();
        assert!(p.account_slug.is_none());
        assert_eq!(p.agent_slug, "agent-y");
    }

    #[test]
    fn peer_ref_empty_is_invalid() {
        assert!(matches!(PeerRef::parse(""), Err(WaitError::InvalidArgs(_))));
    }

    #[test]
    fn peer_ref_malformed_slash() {
        assert!(matches!(
            PeerRef::parse("/agent"),
            Err(WaitError::InvalidArgs(_))
        ));
        assert!(matches!(
            PeerRef::parse("acct/"),
            Err(WaitError::InvalidArgs(_))
        ));
    }

    #[test]
    fn classify_status_codes() {
        assert!(matches!(
            classify_http_err(&anyhow::anyhow!("HTTP 401 unauthorized")),
            WaitError::Unauthorized(_)
        ));
        assert!(matches!(
            classify_http_err(&anyhow::anyhow!("403 forbidden")),
            WaitError::Forbidden(_)
        ));
        assert!(matches!(
            classify_http_err(&anyhow::anyhow!("404 not found")),
            WaitError::InvalidArgs(_)
        ));
        assert!(matches!(
            classify_http_err(&anyhow::anyhow!("connection refused")),
            WaitError::Transient(_)
        ));
    }

    #[test]
    fn invocation_outcome_succeeded_json_shape() {
        let outcome = waiters::InvocationOutcome {
            invocation_id: Uuid::nil(),
            status: "succeeded".into(),
            row: json!({
                "id": "00000000-0000-0000-0000-000000000000",
                "status": "succeeded",
                "capability_name": "message_owner",
                "output_preview": {"reply": "hi"},
            }),
            elapsed_ms: 4200,
        };
        let v = invocation_outcome_json(&outcome);
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["status"], json!("succeeded"));
        assert_eq!(v["capability_name"], json!("message_owner"));
        assert_eq!(v["output"], json!({"reply": "hi"}));
        assert_eq!(v["elapsed_ms"], json!(4200));
    }

    #[test]
    fn invocation_outcome_failed_json_shape() {
        let outcome = waiters::InvocationOutcome {
            invocation_id: Uuid::nil(),
            status: "failed".into(),
            row: json!({
                "id": "00000000-0000-0000-0000-000000000000",
                "status": "failed",
                "capability_name": "echo",
                "error_message": "boom",
            }),
            elapsed_ms: 100,
        };
        let v = invocation_outcome_json(&outcome);
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["error"], json!("boom"));
        assert_eq!(v["status"], json!("failed"));
    }
}
