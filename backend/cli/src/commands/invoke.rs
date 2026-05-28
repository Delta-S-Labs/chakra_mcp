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
    /// Discover the peer, ensure friendship + grant, then invoke.
    /// Returns distinct `waiting_for_friendship` / `waiting_for_grant`
    /// JSON states (exit 3) when prerequisites are missing — pass
    /// `--wait-for-friendship` / `--wait-for-grant` to block instead.
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
    /// Peer to invoke. Either `account-slug/agent-slug` or just
    /// `agent-slug` (must be unambiguous in the network).
    pub peer: String,
    /// Name of the capability to invoke (e.g. `message_owner`).
    pub capability_name: String,
    /// Input — JSON literal, or `@path` to read from a file, or `@-` for stdin.
    pub input_json: String,
    /// Your agent slug — the grantee side that will issue the call.
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
    let as_agent = send
        .as_agent
        .ok_or_else(|| anyhow::anyhow!("--as is required"))?;
    let input = read_json_arg(&send.input)?;
    let body = match (send.grant, send.capability) {
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
    let resp: Value = api.post_relay("/v1/invoke", &body).await?;

    if !send.wait {
        return print(&resp);
    }

    let id = resp
        .get("invocation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("invoke response missing invocation_id"))?
        .to_string();

    eprintln!("Enqueued {id}. Waiting for terminal status…");
    for attempt in 0..LEGACY_POLL_MAX_ATTEMPTS {
        tokio::time::sleep(Duration::from_millis(LEGACY_POLL_INTERVAL_MS)).await;
        let row: Value = api.get_relay(&format!("/v1/invocations/{id}")).await?;
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

    pub(crate) fn matches(&self, account_slug: &str, agent_slug: &str) -> bool {
        if self.agent_slug != agent_slug {
            return false;
        }
        match &self.account_slug {
            Some(acc) => acc == account_slug,
            None => true,
        }
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

/// Look up the peer in the network directory and return their agent id +
/// account/agent slugs. Errors with `InvalidArgs` (exit 6) on ambiguous
/// or missing match — these are user-fixable.
async fn resolve_peer(api: &ApiClient, peer: &PeerRef) -> Result<Value, WaitError> {
    let agents: Value = api
        .get_relay("/v1/network/agents")
        .await
        .map_err(|e| classify_http_err(&e))?;
    let empty = vec![];
    let agents = agents.as_array().unwrap_or(&empty);

    let mut matches: Vec<&Value> = agents
        .iter()
        .filter(|a| {
            let agent_slug = a.get("slug").and_then(|v| v.as_str()).unwrap_or("");
            let account_slug = a.get("account_slug").and_then(|v| v.as_str()).unwrap_or("");
            peer.matches(account_slug, agent_slug)
        })
        .collect();

    match matches.len() {
        0 => Err(WaitError::InvalidArgs(format!(
            "no network-visible agent matched `{}`. Try `chakramcp network` to list.",
            peer.display()
        ))),
        1 => Ok(matches.remove(0).clone()),
        n => {
            let candidates: Vec<String> = matches
                .iter()
                .filter_map(|a| {
                    let acc = a.get("account_slug").and_then(|v| v.as_str())?;
                    let slug = a.get("slug").and_then(|v| v.as_str())?;
                    Some(format!("{acc}/{slug}"))
                })
                .collect();
            Err(WaitError::InvalidArgs(format!(
                "{n} agents matched `{}` — disambiguate with `account/agent` form. Candidates: {}",
                peer.display(),
                candidates.join(", ")
            )))
        }
    }
}

/// Resolve the *my-side* agent slug to a UUID. We need this both to
/// propose friendships and to invoke. The relay's POST endpoints
/// accept UUIDs only.
async fn resolve_my_agent(api: &ApiClient, slug: &str) -> Result<Value, WaitError> {
    let mine: Value = api
        .get_relay("/v1/agents")
        .await
        .map_err(|e| classify_http_err(&e))?;
    let empty = vec![];
    let mine = mine.as_array().unwrap_or(&empty);
    mine.iter()
        .find(|a| {
            a.get("slug")
                .and_then(|v| v.as_str())
                .map(|s| s == slug)
                .unwrap_or(false)
        })
        .cloned()
        .ok_or_else(|| {
            WaitError::InvalidArgs(format!(
                "no agent owned by you with slug `{slug}` — see `chakramcp agents list`"
            ))
        })
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
fn classify_http_err(e: &anyhow::Error) -> WaitError {
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
    let peer_row = resolve_peer(api, &peer_ref)
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

    let my_row = resolve_my_agent(api, &e.from)
        .await
        .map_err(anyhow::Error::from)?;
    let my_agent_id = my_row
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("my agent row missing id"))?
        .to_string();

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

    // 4. Invoke.
    let body = json!({
        "grant_id": grant_id,
        "grantee_agent_id": my_agent_id,
        "input": input,
    });
    let invoke_resp: Value = api
        .post_relay("/v1/invoke", &body)
        .await
        .map_err(|err| anyhow::anyhow!(classify_http_err(&err)))?;
    let invocation_id = invoke_resp
        .get("invocation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("invoke response missing invocation_id"))?
        .to_string();

    // 5. Optionally wait for the invocation to terminate.
    if !e.wait {
        if e.json {
            print(&json!({
                "ok": true,
                "peer": peer_display,
                "friendship": {"status": "accepted", "id": friendship_id},
                "grant": {
                    "capability": e.capability_name,
                    "status": "active",
                    "id": grant_id,
                },
                "invocation": {
                    "id": invocation_id,
                    "status": invoke_resp.get("status"),
                },
            }))?;
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
                let reply = o.row.pointer("/output_preview").cloned();
                let error = o.row.pointer("/error_message").cloned();
                let mut payload = json!({
                    "ok": succeeded,
                    "peer": peer_display,
                    "friendship": {"status": "accepted", "id": friendship_id},
                    "grant": {
                        "capability": e.capability_name,
                        "status": "active",
                        "id": grant_id,
                    },
                    "invocation": {
                        "id": invocation_id,
                        "status": o.status,
                    },
                });
                if succeeded {
                    payload["reply"] = reply.unwrap_or(Value::Null);
                } else {
                    payload["error"] = error.unwrap_or(Value::Null);
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
    fn peer_ref_matches_qualified() {
        let p = PeerRef::parse("acct/agent").unwrap();
        assert!(p.matches("acct", "agent"));
        assert!(!p.matches("acct2", "agent"));
        assert!(!p.matches("acct", "agent2"));
    }

    #[test]
    fn peer_ref_matches_bare_ignores_account() {
        let p = PeerRef::parse("agent").unwrap();
        assert!(p.matches("anywhere", "agent"));
        assert!(p.matches("else", "agent"));
        assert!(!p.matches("anywhere", "other"));
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
