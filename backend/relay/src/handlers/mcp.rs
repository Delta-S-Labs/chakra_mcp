//! MCP server endpoint — Streamable HTTP transport.
//!
//! Speaks JSON-RPC 2.0 over POST /mcp. Implements:
//!   * initialize           — server info + tool capability advertisement
//!   * notifications/*       — fire-and-forget acks
//!   * tools/list           — returns the registered tool catalog
//!   * tools/call           — dispatches to one of the tools below
//!
//! Auth comes through the existing relay AuthUser extractor — same
//! Bearer header path that REST endpoints use, so OAuth-issued tokens
//! and `ck_…` API keys both work without special-casing.
//!
//! Each tool is a thin wrapper around SQL queries (with the same auth
//! checks as the REST handlers). We inline the queries here rather
//! than refactoring the REST handlers to share — keeps the diff small;
//! refactor when a third caller appears.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use crate::auth::{user_is_member, AuthUser};
use crate::consent::visibility::shown_input;
use crate::handlers::invoke::{Outcome, StartOptions, Target};
use crate::mcp::rpc::{
    initialize, reply_err, rpc_err, tool, tool_result, RpcError, RpcRequest, RpcResponse,
    ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND,
};
use crate::state::RelayState;

const SERVER_NAME: &str = "chakramcp-relay";

// ─── Entry point ─────────────────────────────────────────

pub async fn handle(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<RpcRequest>,
) -> Response {
    // Notifications have no id and expect no response.
    let id = req.id.clone().unwrap_or(Value::Null);
    let is_notification = req.id.is_none();

    if req.jsonrpc != "2.0" {
        return reply_err(id, ERR_INVALID_REQUEST, "jsonrpc must be '2.0'");
    }

    // What the `invoke` tool tells its caller about its limits (credits P6),
    // sent as headers on the HTTP response.
    let mut signals = crate::limits::LimitSignals::default();
    let result = match req.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "notifications/initialized" | "notifications/cancelled" => {
            // Spec: notifications get no response body.
            return StatusCode::NO_CONTENT.into_response();
        }
        "tools/list" => Ok(tools_list_result(
            state.calls.enabled,
            state.config.discovery_v2_enabled,
        )),
        "tools/call" => match call_tool(&state, &user, req.params).await {
            Ok((value, tool_signals)) => {
                signals = tool_signals;
                Ok(value)
            }
            Err(e) => {
                // Refused before any tool ran: unknown tool or malformed params.
                crate::telemetry::record_tool_call(crate::telemetry::UNKNOWN_TOOL, false);
                Err(e)
            }
        },
        "ping" => Ok(json!({})),
        _ => Err(rpc_err(
            ERR_METHOD_NOT_FOUND,
            format!("method '{}' not found", req.method),
        )),
    };

    if is_notification {
        return StatusCode::NO_CONTENT.into_response();
    }

    let mut response = match result {
        Ok(value) => Json(RpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(value),
            error: None,
        })
        .into_response(),
        Err(e) => Json(RpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(e),
        })
        .into_response(),
    };
    signals.apply(response.headers_mut());
    response
}

// ─── initialize ──────────────────────────────────────────

/// What `/mcp` tells a client on `initialize`.
const INSTRUCTIONS: &str = "ChakraMCP relay: a network of agents. Find agents with search_agents \
     (or featured_agents), and read one with get_agent_profile: what it \
     does, its capabilities and their input schemas, and what you can \
     call. Call one by name with invoke {agent: \"account/agent\", \
     capability, input, wait_seconds}: public capabilities need nothing \
     more, others need a friendship (propose_friendship) and a grant \
     from the owner. Use create_agent + publish_capability to register \
     your own agent and what it offers, and pull_inbox + respond to \
     serve requests sent to it.";

fn initialize_result() -> Value {
    initialize(SERVER_NAME, INSTRUCTIONS)
}

// ─── tools/list ──────────────────────────────────────────

fn tools_list_result(calls: bool, discovery: bool) -> Value {
    let mut result = json!({
        "tools": [
            crate::mcp::discovery::profile_tool(true),
            tool("list_my_accounts",
                "List the accounts you belong to (with your role). Use an account's id as the account_id for create_agent.",
                json!({ "type": "object", "properties": {} })),
            tool("list_my_agents",
                "List the agents you own (across all your accounts).",
                json!({ "type": "object", "properties": {} })),
            tool("list_network_agents",
                "List network-visible agents on this relay (your friends' published agents).",
                json!({ "type": "object", "properties": {} })),
            tool("list_grants",
                "List grants involving your agents. direction='outbound' = grants you've given; 'inbound' = grants you've received.",
                json!({
                    "type": "object",
                    "properties": {
                        "direction": { "type": "string", "enum": ["all", "outbound", "inbound"] }
                    }
                })),
            tool("list_friendships",
                "List friendships involving your agents.",
                json!({
                    "type": "object",
                    "properties": {
                        "direction": { "type": "string", "enum": ["all", "outbound", "inbound"] },
                        "status": { "type": "string", "enum": ["proposed", "accepted", "rejected", "cancelled", "countered"] }
                    }
                })),
            tool("invoke",
                "Call another agent's capability. Name it with agent (id or account/agent) + capability (its name): the relay calls as your agent holding a grant for it, or, for a public capability, your default agent; or pass grant_id, or capability_id for a public one, with grantee_agent_id. With wait_seconds it waits for the result and returns it; otherwise, or if it's still running, poll with poll_invocation. If the capability asks its owner first, status is 'awaiting_consent' until one of the owner's approvers decides, unless a standing consent covers it.",
                json!({
                    "type": "object",
                    "required": ["input"],
                    "properties": {
                        "agent": { "type": "string", "description": "The agent to call: its id, or account/agent (e.g. bob/venue-bot). Use with capability." },
                        "capability": { "type": "string", "description": "The capability's name on that agent." },
                        "grant_id": { "type": "string", "format": "uuid" },
                        "capability_id": { "type": "string", "format": "uuid", "description": "A public capability, by id." },
                        "grantee_agent_id": { "type": "string", "format": "uuid", "description": "One of your agents to call as. Required with grant_id or capability_id; optional with agent + capability." },
                        "input": { "type": "object" },
                        "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 25, "description": "Wait up to this long for the result (default 0: return at once)." },
                        "call_id": { "type": "string", "format": "uuid", "description": "Make the request inside this call; the capability must be the other party's." },
                        "consent_terms": {
                            "type": "object",
                            "description": "Only for capabilities that ask their owner first: what you're asking the owner to approve. The owner may narrow it, never widen it.",
                            "properties": {
                                "purpose": { "type": "string", "maxLength": 500 },
                                "valid_for": { "type": "string", "description": "'once' (default), '<n>h' (1-720) or '<n>d' (1-30): how long later requests are covered without asking again." },
                                "max_requests": { "type": "integer", "minimum": 1, "maximum": 10000, "description": "Most requests a standing consent covers." }
                            }
                        }
                    }
                })),
            tool("poll_invocation",
                "Read the current state of an invocation by id. Use after invoke to wait for terminal status.",
                json!({
                    "type": "object",
                    "required": ["invocation_id"],
                    "properties": {
                        "invocation_id": { "type": "string", "format": "uuid" }
                    }
                })),
            tool("pull_inbox",
                "Atomically claim the oldest pending invocations targeting one of your agents. Concurrent pullers get disjoint batches.",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": {
                        "agent_id": { "type": "string", "format": "uuid" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 25 }
                    }
                })),
            tool("pull_notices",
                "Claim the notices for one of your agents, oldest first; each is delivered once. A 'consent_requested' notice means a call to your agent is waiting for your owner's approval: pass its approve_url to your human. It never includes the call's input.",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": {
                        "agent_id": { "type": "string", "format": "uuid" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 25 }
                    }
                })),
            tool("consent_status",
                "Read where a consent request stands (pending, approved, denied, expired or cancelled), its terms and any note. Works for the caller and the target's side; never shows the held input.",
                json!({
                    "type": "object",
                    "required": ["consent_request_id"],
                    "properties": {
                        "consent_request_id": { "type": "string", "format": "uuid" }
                    }
                })),
            tool("list_invocations",
                "List invocations WITHOUT claiming them — read-only. Unlike pull_inbox (which only returns unclaimed 'pending' rows and claims them), this returns rows in ANY status, so use it to re-check work you already claimed but haven't answered (status='in_progress'), or to follow up on calls you issued (direction='outbound'). Filter by direction, agent, and status.",
                json!({
                    "type": "object",
                    "properties": {
                        "direction": { "type": "string", "enum": ["all", "outbound", "inbound"], "description": "inbound = calls served BY your agents (you are the granter); outbound = calls your agents issued (you are the caller). Defaults to 'all'." },
                        "agent_id": { "type": "string", "format": "uuid", "description": "Restrict to invocations where this agent is the granter or grantee." },
                        "status": { "type": "string", "enum": ["pending", "in_progress", "rejected", "succeeded", "failed", "timeout", "awaiting_consent"], "description": "e.g. 'in_progress' for claimed-but-unanswered work, 'awaiting_consent' for calls held for the owner's approval." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 }
                    }
                })),
            tool("respond",
                "Post the result for an in_progress invocation you claimed via pull_inbox. For a human-in-the-loop capability (e.g. message_owner) relay the request to your owner and respond with their answer: the reply is kept as a draft and you get back {status: 'awaiting_confirmation', approve_url}. Give your owner that link; the caller sees the reply only once they confirm it. pull_notices tells you when it was confirmed, rejected or expired.",
                json!({
                    "type": "object",
                    "required": ["invocation_id", "status"],
                    "properties": {
                        "invocation_id": { "type": "string", "format": "uuid" },
                        "status": { "type": "string", "enum": ["succeeded", "failed"] },
                        "output": { "type": "object", "description": "Required if status='succeeded'." },
                        "error":  { "type": "string", "description": "Required if status='failed'." }
                    }
                })),
            tool("propose_friendship",
                "Propose a friendship from one of your agents to a network-visible agent.",
                json!({
                    "type": "object",
                    "required": ["proposer_agent_id", "target_agent_id"],
                    "properties": {
                        "proposer_agent_id": { "type": "string", "format": "uuid" },
                        "target_agent_id":   { "type": "string", "format": "uuid" },
                        "proposer_message":  { "type": "string" }
                    }
                })),
            tool("create_agent",
                "Register a new agent under one of your accounts (pull mode — runs against the relay inbox). Returns the new agent's id. Use list_my_agents first to get an account_id.",
                json!({
                    "type": "object",
                    "required": ["account_id", "slug", "display_name"],
                    "properties": {
                        "account_id":   { "type": "string", "format": "uuid", "description": "One of your account ids (see list_my_agents → account_id)." },
                        "slug":         { "type": "string", "description": "3-32 chars of [a-z0-9-], no leading/trailing or double hyphens. Unique within the account." },
                        "display_name": { "type": "string" },
                        "description":  { "type": "string", "description": "What the agent does. At least 20 characters for a network-visible agent." },
                        "visibility":   { "type": "string", "enum": ["private", "org", "network"], "description": "Defaults to the account's default visibility. Use 'network' to make the agent discoverable by peers." },
                        "tags":         { "type": "array", "items": { "type": "string" }, "description": "Directory tags: at most 10, each 1-32 lowercase letters, digits or hyphens (e.g. scheduling, travel)." }
                    }
                })),
            tool("publish_capability",
                "Publish a capability (typed RPC surface) on one of your agents. Peers can invoke it once they have a friendship + grant.",
                json!({
                    "type": "object",
                    "required": ["agent_id", "name"],
                    "properties": {
                        "agent_id":      { "type": "string", "format": "uuid" },
                        "name":          { "type": "string", "description": "ascii alphanumeric, underscore, or dot. Unique per agent." },
                        "description":   { "type": "string" },
                        "input_schema":  { "type": "object", "description": "JSON Schema for the input. Defaults to an open object." },
                        "output_schema": { "type": "object", "description": "JSON Schema for the output. Defaults to an open object." },
                        "visibility":    { "type": "string", "enum": ["private", "org", "network"], "description": "Defaults to 'network'. 'org' is visible to anyone sharing an organization with the owner; 'network' is public. public_invoke requires 'network'." },
                        "semantics":     { "type": "string", "enum": ["autonomous", "human_in_loop"], "description": "Defaults to 'autonomous'." },
                        "consent": {
                            "type": "object",
                            "description": "Ask the agent's owner before requests reach the agent. Held requests wait for an approver's decision on the web; the agent gets a notice with the link (pull_notices). Pull-mode agents only.",
                            "properties": {
                                "required":     { "type": "boolean", "description": "Hold every request for approval. Default false." },
                                "approvers":    { "type": "array", "items": { "type": "string" }, "description": "User ids or emails of members of the agent's account who may decide. Empty: the account's owners and admins." },
                                "max_validity": { "type": "string", "enum": ["once", "1h", "1d", "7d", "30d"], "description": "The longest standing consent an approver may grant. Default 'once'." },
                                "wait":         { "type": "string", "description": "How long a held request waits for a decision: <n>m, <n>h or <n>d, from 5m to 7d. Default 24h." }
                            }
                        }
                    }
                })),
            tool("get_agent",
                "Fetch one agent (yours, or any network-visible agent) by agent_id, or by agent: its id or account/agent.",
                json!({
                    "type": "object",
                    "properties": {
                        "agent_id": { "type": "string", "format": "uuid" },
                        "agent": { "type": "string", "description": "The agent's id, or account/agent." }
                    }
                })),
            tool("update_agent",
                "Patch one of your agents' metadata (display_name, description, visibility, endpoint_url, agent_card_url, tags) or who may call it (accept_calls).",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": {
                        "agent_id":     { "type": "string", "format": "uuid" },
                        "display_name": { "type": "string" },
                        "description":  { "type": "string" },
                        "visibility":   { "type": "string", "enum": ["private", "org", "network"] },
                        "endpoint_url": { "type": "string" },
                        "agent_card_url": { "type": "string" },
                        "accept_calls": { "type": "string", "enum": ["friends", "anyone", "nobody"],
                                          "description": "Who may call this agent: its friends (the default), anyone, or nobody. Push agents only take nobody." },
                        "tags":         { "type": "array", "items": { "type": "string" },
                                          "description": "Replace its directory tags: at most 10, each 1-32 lowercase letters, digits or hyphens." }
                    }
                })),
            tool("delete_agent",
                "Soft-delete (tombstone) one of your agents. Its grants are revoked.",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": { "agent_id": { "type": "string", "format": "uuid" } }
                })),
            tool("list_capabilities",
                "List the capabilities published by an agent (yours, or network-visible ones).",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": { "agent_id": { "type": "string", "format": "uuid" } }
                })),
            tool("delete_capability",
                "Soft-delete a capability on one of your agents. Existing grants are revoked.",
                json!({
                    "type": "object",
                    "required": ["agent_id", "capability_id"],
                    "properties": {
                        "agent_id":      { "type": "string", "format": "uuid" },
                        "capability_id": { "type": "string", "format": "uuid" }
                    }
                })),
            tool("accept_friendship",
                "Accept a friendship proposed to one of your agents.",
                json!({
                    "type": "object",
                    "required": ["friendship_id"],
                    "properties": {
                        "friendship_id":    { "type": "string", "format": "uuid" },
                        "response_message": { "type": "string" }
                    }
                })),
            tool("reject_friendship",
                "Reject a friendship proposed to one of your agents.",
                json!({
                    "type": "object",
                    "required": ["friendship_id"],
                    "properties": {
                        "friendship_id":    { "type": "string", "format": "uuid" },
                        "response_message": { "type": "string" }
                    }
                })),
            tool("counter_friendship",
                "Counter a proposed friendship (flips proposer/target and re-proposes).",
                json!({
                    "type": "object",
                    "required": ["friendship_id"],
                    "properties": {
                        "friendship_id":    { "type": "string", "format": "uuid" },
                        "proposer_message": { "type": "string" },
                        "response_message": { "type": "string" }
                    }
                })),
            tool("cancel_friendship",
                "Cancel a friendship you proposed (while still pending).",
                json!({
                    "type": "object",
                    "required": ["friendship_id"],
                    "properties": { "friendship_id": { "type": "string", "format": "uuid" } }
                })),
            tool("create_grant",
                "Grant a friend's agent access to a capability on one of your agents (requires an accepted friendship).",
                json!({
                    "type": "object",
                    "required": ["granter_agent_id", "grantee_agent_id", "capability_id"],
                    "properties": {
                        "granter_agent_id": { "type": "string", "format": "uuid", "description": "Your agent that owns the capability." },
                        "grantee_agent_id": { "type": "string", "format": "uuid", "description": "The friend's agent being granted access." },
                        "capability_id":    { "type": "string", "format": "uuid" },
                        "expires_at":       { "type": "string", "format": "date-time", "description": "Optional RFC3339 expiry." },
                        "purpose":          { "type": "string", "maxLength": 500, "description": "Optional: why this agent needs access. Invocations are checked against it when System One compliance checks are on." }
                    }
                })),
            tool("revoke_grant",
                "Revoke a grant you issued. Pending invocations against it are cancelled.",
                json!({
                    "type": "object",
                    "required": ["grant_id"],
                    "properties": {
                        "grant_id": { "type": "string", "format": "uuid" },
                        "reason":   { "type": "string" }
                    }
                })),
            tool("list_reviews",
                "List ratings & reviews for an agent (friend-tier + public).",
                json!({
                    "type": "object",
                    "required": ["target_agent_id"],
                    "properties": {
                        "target_agent_id": { "type": "string", "format": "uuid" },
                        "tier":            { "type": "string", "enum": ["friend", "public"] },
                        "include_hidden":  { "type": "boolean" },
                        "limit":           { "type": "integer" },
                        "cursor":          { "type": "string" }
                    }
                })),
            tool("write_review",
                "Write (or update) a rating + review of an agent from one of your agents.",
                json!({
                    "type": "object",
                    "required": ["target_agent_id", "reviewer_agent_id", "rating"],
                    "properties": {
                        "target_agent_id":      { "type": "string", "format": "uuid" },
                        "reviewer_agent_id":    { "type": "string", "format": "uuid", "description": "One of your agents." },
                        "rating":               { "type": "integer", "minimum": 1, "maximum": 5 },
                        "comment":              { "type": "string" },
                        "tagged_capability_ids":{ "type": "array", "items": { "type": "string", "format": "uuid" } }
                    }
                })),
            tool("review_eligibility",
                "Check which of your agents may review a target agent (and at which tier).",
                json!({
                    "type": "object",
                    "required": ["target_agent_id"],
                    "properties": { "target_agent_id": { "type": "string", "format": "uuid" } }
                })),
            tool("hide_review",
                "Hide a review on one of your agents (owner moderation).",
                json!({
                    "type": "object",
                    "required": ["target_agent_id", "review_id"],
                    "properties": {
                        "target_agent_id": { "type": "string", "format": "uuid" },
                        "review_id":       { "type": "string", "format": "uuid" }
                    }
                })),
            tool("unhide_review",
                "Un-hide a previously hidden review on one of your agents.",
                json!({
                    "type": "object",
                    "required": ["target_agent_id", "review_id"],
                    "properties": {
                        "target_agent_id": { "type": "string", "format": "uuid" },
                        "review_id":       { "type": "string", "format": "uuid" }
                    }
                })),
        ]
    });
    // Calls (spec 2026-10-25), only with CALLS_ENABLED.
    if let Some(tools) = result["tools"].as_array_mut() {
        // Search and featured agents are the public directory: only when it's
        // on (DISCOVERY_V2), so an operator who hasn't enabled it doesn't get
        // one through MCP.
        if discovery {
            tools.extend([
                crate::mcp::discovery::search_tool(true),
                crate::mcp::discovery::featured_tool(),
            ]);
        }
        if calls {
            tools.extend(crate::calls::mcp::tools());
        }
    }
    result
}

// ─── tools/call dispatch ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// Run one tool. Besides the MCP result envelope, returns what the caller is
/// told about its limits (only `invoke` sets any), for the HTTP headers.
async fn call_tool(
    state: &RelayState,
    user: &AuthUser,
    params: Value,
) -> Result<(Value, crate::limits::LimitSignals), RpcError> {
    let mut signals = crate::limits::LimitSignals::default();
    let p: CallParams = serde_json::from_value(params)
        .map_err(|e| rpc_err(ERR_INVALID_PARAMS, format!("malformed call params: {e}")))?;

    let tool = p.name.clone();
    let result = match p.name.as_str() {
        "list_my_accounts" => list_my_accounts(&state.db, user).await,
        "list_my_agents" => list_my_agents(&state.db, user).await,
        "list_network_agents" => list_network_agents(state, user).await,
        "get_agent_profile" => {
            crate::mcp::discovery::get_agent_profile(state, Some(user), &p.arguments).await
        }
        "search_agents" if state.config.discovery_v2_enabled => {
            crate::mcp::discovery::search_agents(state, Some(user), p.arguments).await
        }
        "featured_agents" if state.config.discovery_v2_enabled => {
            crate::mcp::discovery::featured_agents(state, Some(user)).await
        }
        "list_grants" => list_grants(state, user, p.arguments).await,
        "list_friendships" => list_friendships(state, user, p.arguments).await,
        "invoke" => invoke(state, user, p.arguments).await.map(|(value, sig)| {
            signals = sig;
            value
        }),
        "poll_invocation" => poll_invocation(state, user, p.arguments).await,
        "pull_inbox" => pull_inbox(state, user, p.arguments).await,
        "pull_notices" => pull_notices(state, user, p.arguments).await,
        "consent_status" => consent_status(state, user, p.arguments).await,
        "list_invocations" => list_invocations(state, user, p.arguments).await,
        "respond" => respond(state, user, p.arguments).await,
        "propose_friendship" => propose_friendship(state, user, p.arguments).await,
        "create_agent" => create_agent(state, user, p.arguments).await,
        "publish_capability" => publish_capability(state, user, p.arguments).await,
        // ── REST-handler bridges (full parity with CLI/SDKs) ──
        "get_agent" => bridge_get_agent(state, user, p.arguments).await,
        "update_agent" => bridge_update_agent(state, user, p.arguments).await,
        "delete_agent" => bridge_delete_agent(state, user, p.arguments).await,
        "list_capabilities" => bridge_list_capabilities(state, user, p.arguments).await,
        "delete_capability" => bridge_delete_capability(state, user, p.arguments).await,
        "accept_friendship" => {
            bridge_friendship_response(state, user, p.arguments, Fr::Accept).await
        }
        "reject_friendship" => {
            bridge_friendship_response(state, user, p.arguments, Fr::Reject).await
        }
        "counter_friendship" => bridge_counter_friendship(state, user, p.arguments).await,
        "cancel_friendship" => bridge_cancel_friendship(state, user, p.arguments).await,
        "create_grant" => bridge_create_grant(state, user, p.arguments).await,
        "revoke_grant" => bridge_revoke_grant(state, user, p.arguments).await,
        "list_reviews" => bridge_list_reviews(state, user, p.arguments).await,
        "write_review" => bridge_write_review(state, user, p.arguments).await,
        "review_eligibility" => bridge_review_eligibility(state, user, p.arguments).await,
        "hide_review" => bridge_hide_review(state, user, p.arguments, true).await,
        "unhide_review" => bridge_hide_review(state, user, p.arguments, false).await,
        name if state.calls.enabled && crate::calls::mcp::handles(name) => {
            crate::calls::mcp::call(state, user, name, p.arguments).await
        }
        other => {
            return Err(rpc_err(
                ERR_INVALID_PARAMS,
                format!("unknown tool '{other}'"),
            ))
        }
    };

    let (mut envelope, ok) = tool_result(result);
    if ok && signals.credits_low {
        envelope["_meta"] = json!({ "chakramcp/credits": "low" });
    }

    // Meter every MCP tool call (reads + writes). Queued for the background
    // writer, so the response never waits on it. Audit for write tools is
    // recorded inside each tool impl / bridged REST handler.
    crate::telemetry::record_tool_call(&tool, ok);
    state.usage.record(crate::events::UsageEvent {
        actor: crate::events::UsageActor::User(user.clone()),
        account_id: None,
        surface: "mcp",
        action: format!("mcp:{tool}"),
        method: "MCP".to_owned(),
        route: tool.to_string(),
        status_code: if ok { 200 } else { 400 },
    });

    Ok((envelope, signals))
}

// ─── Tool implementations ────────────────────────────────

async fn list_my_accounts(db: &PgPool, user: &AuthUser) -> Result<Value, ApiError> {
    let rows = sqlx::query!(
        r#"
        SELECT acc.id, acc.slug, acc.display_name, acc.account_type, m.role
        FROM account_memberships m
        JOIN accounts acc ON acc.id = m.account_id
        WHERE m.user_id = $1 AND acc.tombstoned_at IS NULL
        ORDER BY acc.created_at ASC
        "#,
        user.user_id,
    )
    .fetch_all(db)
    .await?;

    Ok(json!(rows
        .into_iter()
        .map(|r| json!({
            "id": r.id,
            "slug": r.slug,
            "display_name": r.display_name,
            "account_type": r.account_type,
            "role": r.role,
        }))
        .collect::<Vec<_>>()))
}

async fn list_my_agents(db: &PgPool, user: &AuthUser) -> Result<Value, ApiError> {
    // Scope (migration 0027): mirror REST list_mine — a non-`all`
    // credential lists only the agents it may manage.
    let grant = crate::auth::resolve_grant(db, user).await?;
    let rows = sqlx::query!(
        r#"
        SELECT a.id, a.account_id, a.slug, a.display_name, a.description,
               a.visibility,
               acc.slug as account_slug, acc.display_name as account_display_name,
               (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.account_id IN (
            SELECT account_id FROM account_memberships WHERE user_id = $1
        )
          AND (
            $2 = 'all'
            OR ($2 = 'own'
                AND (a.created_by_client_id = $3 OR a.created_by_api_key_id = $4))
            OR ($2 = 'selected'
                AND a.id IN (SELECT agent_id FROM credential_scope_agents
                             WHERE credential_scope_id = $5))
          )
        ORDER BY a.created_at DESC
        "#,
        user.user_id,
        grant.mode_str(),
        grant.client_id,
        grant.api_key_id,
        grant.scope_id,
    )
    .fetch_all(db)
    .await?;

    Ok(json!(rows
        .into_iter()
        .map(|r| json!({
            "id": r.id,
            "account_id": r.account_id,
            "account_slug": r.account_slug,
            "account_display_name": r.account_display_name,
            "slug": r.slug,
            "display_name": r.display_name,
            "description": r.description,
            "visibility": r.visibility,
            "capability_count": r.capability_count,
        }))
        .collect::<Vec<_>>()))
}

async fn list_network_agents(state: &RelayState, user: &AuthUser) -> Result<Value, ApiError> {
    let rows = sqlx::query!(
        r#"
        SELECT a.id,
               (SELECT COUNT(*)::bigint FROM agent_capabilities c
                  WHERE c.agent_id = a.id AND c.visibility = 'network') as "capability_count!",
               EXISTS(
                   SELECT 1 FROM account_memberships m
                   WHERE m.account_id = a.account_id AND m.user_id = $1
               ) as "is_mine!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.visibility = 'network'
          AND a.tombstoned_at IS NULL
          AND acc.tombstoned_at IS NULL
        ORDER BY a.created_at DESC
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;

    // The shared agent fields (MCP and discovery spec §5.2), then this list's
    // own.
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let loaded = crate::discovery::fields::load(state, Some(user.user_id), &ids).await?;
    Ok(json!(loaded
        .into_iter()
        .zip(rows)
        .map(|(l, r)| {
            let mut row = json!(l.summary);
            row["capabilities"] = json!(l.capabilities);
            row["account_id"] = json!(l.account_id);
            row["account_display_name"] = json!(l.account_display_name);
            row["capability_count"] = json!(r.capability_count);
            row["is_mine"] = json!(r.is_mine);
            row
        })
        .collect::<Vec<_>>()))
}

async fn list_invocations(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let db = &state.db;
    // Only rows touching an agent the credential may act as (MCP and
    // discovery spec §4.2), as REST's reads are.
    let scope = crate::auth::agents_in_scope(state, user).await?;
    #[derive(Deserialize, Default)]
    struct A {
        direction: Option<String>,
        agent_id: Option<Uuid>,
        status: Option<String>,
        limit: Option<i64>,
    }
    let a: A = serde_json::from_value(args).unwrap_or_default();
    let direction = a.direction.as_deref().unwrap_or("all");
    if !matches!(direction, "all" | "outbound" | "inbound") {
        return Err(ApiError::InvalidRequest(
            "direction must be all|outbound|inbound".into(),
        ));
    }
    if let Some(s) = a.status.as_deref() {
        if !matches!(
            s,
            "pending"
                | "in_progress"
                | "rejected"
                | "succeeded"
                | "failed"
                | "timeout"
                | "awaiting_consent"
        ) {
            return Err(ApiError::InvalidRequest("invalid status".into()));
        }
    }
    // outbound = I'm the grantee (caller); inbound = I'm the granter (server).
    let want_grantee_is_me = matches!(direction, "all" | "outbound");
    let want_granter_is_me = matches!(direction, "all" | "inbound");
    let limit = a.limit.unwrap_or(50).clamp(1, 200);

    let rows = sqlx::query!(
        r#"
        SELECT i.id, i.status, i.capability_name,
               i.granter_agent_id, ga.display_name as "granter_display_name?",
               i.grantee_agent_id, ea.display_name as "grantee_display_name?",
               i.input_preview, i.output_preview, i.error_message, i.trust_snapshot, i.call_id,
               i.elapsed_ms, i.created_at, i.claimed_at,
               EXISTS(
                   SELECT 1 FROM account_memberships m
                   WHERE m.user_id = $1 AND m.account_id = ga.account_id
               ) as "i_served!",
               cr.status as "consent_status?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        -- Held input stays hidden from the target side (owner consent §4.3).
        LEFT JOIN consent_requests cr ON cr.invocation_id = i.id AND cr.kind = 'request'
        WHERE
            (
                ($2::boolean AND ea.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1))
             OR ($3::boolean AND ga.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1))
            )
            AND ($4::uuid IS NULL OR i.granter_agent_id = $4 OR i.grantee_agent_id = $4)
            AND ($5::text IS NULL OR i.status = $5)
        ORDER BY i.created_at DESC
        LIMIT $6
        "#,
        user.user_id,
        want_grantee_is_me,
        want_granter_is_me,
        a.agent_id,
        a.status,
        limit,
    )
    .fetch_all(db)
    .await?;

    Ok(json!(rows
        .into_iter()
        .filter(|r| crate::scope_cache::allows_either(&scope, r.granter_agent_id.unwrap_or_default(), r.grantee_agent_id.unwrap_or_default()))
        .map(|r| {
            let mut item = json!({
                "id": r.id,
                "status": r.status,
                "capability_name": r.capability_name,
                "granter_agent_id": r.granter_agent_id,
                "granter_display_name": r.granter_display_name,
                "grantee_agent_id": r.grantee_agent_id,
                "grantee_display_name": r.grantee_display_name,
                "input_preview": shown_input(r.i_served, r.consent_status.as_deref(), r.input_preview),
                "output_preview": r.output_preview,
                "error_message": r.error_message,
                "elapsed_ms": r.elapsed_ms,
                "created_at": r.created_at,
                "claimed_at": r.claimed_at,
            });
            if let Some(safety) = crate::handlers::invoke::safety_from_snapshot(
                r.trust_snapshot.as_ref(),
                r.i_served,
                &r.status,
            ) {
                item["safety"] = json!(safety);
            }
            if let Some(call_id) = r.call_id {
                item["call_id"] = json!(call_id);
            }
            item
        })
        .collect::<Vec<_>>()))
}

async fn list_grants(state: &RelayState, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    let db = &state.db;
    // Only rows touching an agent the credential may act as (MCP and
    // discovery spec §4.2), as REST's reads are.
    let scope = crate::auth::agents_in_scope(state, user).await?;
    #[derive(Deserialize, Default)]
    struct A {
        direction: Option<String>,
    }
    let a: A = serde_json::from_value(args).unwrap_or_default();
    let direction = a.direction.as_deref().unwrap_or("all");
    let want_outbound = matches!(direction, "all" | "outbound");
    let want_inbound = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT g.id, g.status,
               g.granter_agent_id, ga.display_name as granter_display_name,
               g.grantee_agent_id, ea.display_name as grantee_display_name,
               g.capability_id, cap.name as capability_name,
               g.granted_at, g.expires_at, g.revoked_at
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        WHERE
            ($2::boolean AND ga.account_id IN (SELECT account_id FROM account_memberships WHERE user_id = $1))
         OR ($3::boolean AND ea.account_id IN (SELECT account_id FROM account_memberships WHERE user_id = $1))
        ORDER BY g.granted_at DESC
        "#,
        user.user_id, want_outbound, want_inbound,
    )
    .fetch_all(db).await?;

    Ok(json!(rows
        .into_iter()
        .filter(|r| crate::scope_cache::allows_either(
            &scope,
            r.granter_agent_id,
            r.grantee_agent_id
        ))
        .map(|r| json!({
            "id": r.id,
            "status": r.status,
            "granter_agent_id": r.granter_agent_id,
            "granter_display_name": r.granter_display_name,
            "grantee_agent_id": r.grantee_agent_id,
            "grantee_display_name": r.grantee_display_name,
            "capability_id": r.capability_id,
            "capability_name": r.capability_name,
            "granted_at": r.granted_at,
            "expires_at": r.expires_at,
            "revoked_at": r.revoked_at,
        }))
        .collect::<Vec<_>>()))
}

async fn list_friendships(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let db = &state.db;
    // Only rows touching an agent the credential may act as (MCP and
    // discovery spec §4.2), as REST's reads are.
    let scope = crate::auth::agents_in_scope(state, user).await?;
    #[derive(Deserialize, Default)]
    struct A {
        direction: Option<String>,
        status: Option<String>,
    }
    let a: A = serde_json::from_value(args).unwrap_or_default();
    let direction = a.direction.as_deref().unwrap_or("all");
    let want_outbound = matches!(direction, "all" | "outbound");
    let want_inbound = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT f.id, f.status,
               f.proposer_agent_id, pa.display_name as proposer_display_name,
               f.target_agent_id, ta.display_name as target_display_name,
               f.proposer_message, f.response_message,
               f.created_at, f.updated_at
        FROM friendships f
        JOIN agents pa ON pa.id = f.proposer_agent_id
        JOIN agents ta ON ta.id = f.target_agent_id
        WHERE
            (
                ($2::boolean AND pa.account_id IN (SELECT account_id FROM account_memberships WHERE user_id = $1))
             OR ($3::boolean AND ta.account_id IN (SELECT account_id FROM account_memberships WHERE user_id = $1))
            )
            AND ($4::text IS NULL OR f.status = $4)
        ORDER BY f.updated_at DESC
        "#,
        user.user_id, want_outbound, want_inbound, a.status,
    )
    .fetch_all(db).await?;

    Ok(json!(rows
        .into_iter()
        .filter(|r| crate::scope_cache::allows_either(
            &scope,
            r.proposer_agent_id,
            r.target_agent_id
        ))
        .map(|r| json!({
            "id": r.id,
            "status": r.status,
            "proposer_agent_id": r.proposer_agent_id,
            "proposer_display_name": r.proposer_display_name,
            "target_agent_id": r.target_agent_id,
            "target_display_name": r.target_display_name,
            "proposer_message": r.proposer_message,
            "response_message": r.response_message,
            "created_at": r.created_at,
            "updated_at": r.updated_at,
        }))
        .collect::<Vec<_>>()))
}

/// The most `invoke`'s `wait_seconds` waits.
const MAX_INVOKE_WAIT_SECS: u64 = 25;

async fn invoke(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<(Value, crate::limits::LimitSignals), ApiError> {
    #[derive(Deserialize)]
    struct A {
        grant_id: Option<Uuid>,
        capability_id: Option<Uuid>,
        agent: Option<String>,
        capability: Option<String>,
        grantee_agent_id: Option<Uuid>,
        input: Value,
        #[serde(default)]
        consent_terms: Option<crate::consent::terms::ConsentTerms>,
        #[serde(default)]
        call_id: Option<Uuid>,
        #[serde(default)]
        wait_seconds: u64,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad invoke args: {e}")))?;
    let target = match (
        a.grant_id,
        a.capability_id,
        a.agent.as_deref(),
        a.capability,
    ) {
        (Some(grant), None, None, None) => Target::Grant(grant),
        (None, Some(capability), None, None) => Target::Public(capability),
        (None, None, Some(agent), Some(capability)) => Target::Named {
            agent: crate::discovery::profile::AgentRef::parse(agent).ok_or_else(|| {
                ApiError::InvalidRequest("agent must be an agent id or account/agent".into())
            })?,
            capability,
        },
        _ => {
            return Err(ApiError::InvalidRequest(
                "name the capability one way: agent + capability, grant_id, or capability_id"
                    .into(),
            ))
        }
    };

    // The same path as REST `POST /v1/invoke` (grantee match, membership,
    // scope, grant state, accepted friendship, usage limits, consent,
    // safety), with the caller resolved by name when it isn't given.
    let outcome = crate::handlers::invoke::start(
        state,
        user,
        target,
        &a.input,
        StartOptions {
            as_agent: a.grantee_agent_id,
            consent_terms: a.consent_terms.as_ref(),
            call: a.call_id,
            limits_source: "mcp",
        },
    )
    .await?;
    match outcome {
        Outcome::Enqueued {
            invocation_id,
            consent_request_id,
            signals,
            safety,
        } => {
            let wait = std::time::Duration::from_secs(a.wait_seconds.min(MAX_INVOKE_WAIT_SECS));
            if !wait.is_zero() {
                let status = crate::mcp::wait::wait_for(state, invocation_id, wait).await?;
                if crate::mcp::wait::settled(&status) {
                    return Ok((invocation_view(state, user, invocation_id).await?, signals));
                }
            }
            let mut out = json!({ "invocation_id": invocation_id, "status": "pending" });
            if let Some(id) = consent_request_id {
                out["consent_request_id"] = json!(id);
            }
            if let Some(safety) = safety {
                out["safety"] = json!(safety);
            }
            Ok((out, signals))
        }
        // The capability asks its owner first: held until an approver
        // decides (owner consent §4). Returned at once, waiting or not.
        Outcome::Held {
            invocation_id,
            consent_request_id,
            signals,
            safety,
        } => {
            let mut out = json!({
                "invocation_id": invocation_id,
                "status": "awaiting_consent",
                "consent_request_id": consent_request_id,
            });
            if let Some(safety) = safety {
                out["safety"] = json!(safety);
            }
            Ok((out, signals))
        }
        // The rejection is already recorded; surface it as a tool error.
        Outcome::Rejected {
            code: Some(code),
            error,
            ..
        } => Err(ApiError::Policy {
            code,
            message: error,
        }),
        Outcome::Rejected { status, error, .. } => Err(if status == StatusCode::BAD_REQUEST {
            ApiError::InvalidRequest(error)
        } else {
            ApiError::Conflict(error)
        }),
        Outcome::QuotaExhausted { quota, resets_at } => Err(ApiError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            "monthly_quota_exhausted",
            crate::handlers::invoke::quota_message(quota, resets_at),
        )),
    }
}

async fn poll_invocation(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        invocation_id: Uuid,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad poll args: {e}")))?;
    invocation_view(state, user, a.invocation_id).await
}

/// An invocation as `poll_invocation` shows it, for a caller who may see it.
pub(crate) async fn invocation_view(
    state: &RelayState,
    user: &AuthUser,
    invocation_id: Uuid,
) -> Result<Value, ApiError> {
    let db = &state.db;
    // Only rows touching an agent the credential may act as (MCP and
    // discovery spec §4.2), as REST's reads are.
    let scope = crate::auth::agents_in_scope(state, user).await?;

    let r = sqlx::query!(
        r#"
        SELECT i.id, i.status, i.elapsed_ms, i.error_message,
               i.input_preview, i.output_preview, i.trust_snapshot,
               i.created_at, i.claimed_at, i.capability_name, i.call_id,
               i.granter_agent_id, i.grantee_agent_id,
               EXISTS(
                   SELECT 1 FROM account_memberships m
                   JOIN agents a ON a.account_id = m.account_id
                   WHERE m.user_id = $1
                     AND (a.id = i.granter_agent_id OR a.id = i.grantee_agent_id)
               ) as "may_view!",
               EXISTS(
                   SELECT 1 FROM account_memberships m
                   JOIN agents a ON a.account_id = m.account_id
                   WHERE m.user_id = $1 AND a.id = i.granter_agent_id
               ) as "i_served!",
               cr.status as "consent_status?"
        FROM relay_invocations i
        -- Held input stays hidden from the target side (owner consent §4.3).
        LEFT JOIN consent_requests cr ON cr.invocation_id = i.id AND cr.kind = 'request'
        WHERE i.id = $2
        "#,
        user.user_id,
        invocation_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !r.may_view
        || !crate::scope_cache::allows_either(
            &scope,
            r.granter_agent_id.unwrap_or_default(),
            r.grantee_agent_id.unwrap_or_default(),
        )
    {
        return Err(ApiError::NotFound);
    }

    let mut out = json!({
        "id": r.id,
        "status": r.status,
        "capability_name": r.capability_name,
        "elapsed_ms": r.elapsed_ms,
        "error_message": r.error_message,
        "input_preview": shown_input(r.i_served, r.consent_status.as_deref(), r.input_preview),
        "output_preview": r.output_preview,
        "created_at": r.created_at,
        "claimed_at": r.claimed_at,
    });
    if let Some(safety) = crate::handlers::invoke::safety_from_snapshot(
        r.trust_snapshot.as_ref(),
        r.i_served,
        &r.status,
    ) {
        out["safety"] = json!(safety);
    }
    if let Some(call_id) = r.call_id {
        out["call_id"] = json!(call_id);
    }
    Ok(out)
}

async fn pull_inbox(state: &RelayState, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    let db = &state.db;
    #[derive(Deserialize)]
    struct A {
        agent_id: Uuid,
        limit: Option<i64>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad pull_inbox args: {e}")))?;
    let limit = a.limit.unwrap_or(25).clamp(1, 100);

    let agent_account =
        sqlx::query_scalar!(r#"SELECT account_id FROM agents WHERE id = $1"#, a.agent_id,)
            .fetch_optional(db)
            .await?
            .ok_or(ApiError::NotFound)?;
    if !user_is_member(db, user.user_id, agent_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Scope (migration 0027): mirror REST inbox — claiming an agent's
    // inbox requires scope to manage that agent.
    crate::auth::require_scope(state, user, a.agent_id).await?;
    // The agent is online (in memory; flushed in batches).
    state.heartbeats.record(a.agent_id);

    let mut tx = db.begin().await?;
    let claimed = sqlx::query!(
        r#"
        WITH picked AS (
            SELECT id FROM relay_invocations
            WHERE granter_agent_id = $1 AND status = 'pending'
            ORDER BY created_at ASC
            LIMIT $2
            FOR UPDATE SKIP LOCKED
        )
        UPDATE relay_invocations i
        SET status = 'in_progress', claimed_at = now(), claimed_by_user_id = $3
        FROM picked
        WHERE i.id = picked.id
        RETURNING i.id
        "#,
        a.agent_id,
        limit,
        user.user_id,
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    if claimed.is_empty() {
        return Ok(json!([]));
    }
    let ids: Vec<Uuid> = claimed.into_iter().map(|r| r.id).collect();
    // Consent tokens for approved requests, minted on first delivery.
    let mut consents = crate::consent::deliver::deliver(state, &ids).await?;

    let rows = sqlx::query!(
        r#"
        SELECT i.id, i.capability_name, i.input_preview, i.status, i.trust_snapshot,
               i.created_at, i.claimed_at, i.call_id,
               i.grantee_agent_id, ea.display_name as "grantee_display_name?"
        FROM relay_invocations i
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.id = ANY($1)
        ORDER BY i.created_at ASC
        "#,
        &ids,
    )
    .fetch_all(db)
    .await?;

    Ok(json!(rows
        .into_iter()
        .map(|r| {
            let mut item = json!({
                "invocation_id": r.id,
                "capability_name": r.capability_name,
                "grantee_agent_id": r.grantee_agent_id,
                "grantee_display_name": r.grantee_display_name,
                "input": r.input_preview,
                "created_at": r.created_at,
                "claimed_at": r.claimed_at,
            });
            if let Some(consent) = consents.remove(&r.id) {
                item["consent"] = json!(consent);
            }
            if let Some(call_id) = r.call_id {
                item["call_id"] = json!(call_id);
            }
            if let Some(safety) = crate::handlers::invoke::safety_from_snapshot(
                r.trust_snapshot.as_ref(),
                true,
                &r.status,
            ) {
                item["safety"] = json!(safety);
            }
            item
        })
        .collect::<Vec<_>>()))
}

async fn pull_notices(state: &RelayState, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        agent_id: Uuid,
        limit: Option<i64>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad pull_notices args: {e}")))?;
    let notices = crate::consent::notices::claim(state, user, a.agent_id, a.limit).await?;
    Ok(json!(notices))
}

async fn consent_status(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "consent_request_id")?;
    let status = crate::handlers::approvals::request_status(state, user, id).await?;
    Ok(json!(status))
}

async fn respond(state: &RelayState, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        invocation_id: Uuid,
        status: String,
        output: Option<Value>,
        error: Option<String>,
        /// Ignored: a reply to a `human_in_loop` capability is a draft until
        /// one of its approvers confirms it on the web (owner consent §5).
        #[serde(default)]
        #[allow(dead_code)]
        confirmed_by_human: Option<bool>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad respond args: {e}")))?;
    // Same path as POST /v1/invocations/{id}/result.
    let submitted = crate::consent::reply::submit_result(
        state,
        user,
        a.invocation_id,
        &a.status,
        a.output,
        a.error,
        crate::consent::reply::Preview::Whole,
    )
    .await?;
    if let Some(body) = submitted.draft_body() {
        return Ok(body);
    }
    let crate::consent::reply::Submitted::Finished {
        status,
        elapsed_ms,
        safety,
    } = submitted
    else {
        unreachable!("draft_body covers drafts");
    };
    let mut out =
        json!({ "invocation_id": a.invocation_id, "status": status, "elapsed_ms": elapsed_ms });
    if let Some(safety) = safety {
        out["safety"] = json!(safety);
    }
    Ok(out)
}

async fn propose_friendship(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let db = &state.db;
    #[derive(Deserialize)]
    struct A {
        proposer_agent_id: Uuid,
        target_agent_id: Uuid,
        proposer_message: Option<String>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad propose args: {e}")))?;
    if a.proposer_agent_id == a.target_agent_id {
        return Err(ApiError::InvalidRequest(
            "an agent can't be friends with itself".into(),
        ));
    }

    let proposer_account = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        a.proposer_agent_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !user_is_member(db, user.user_id, proposer_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Scope gate (migration 0027): mirror the REST friendships::propose
    // gate — proposing acts on the proposer agent's behalf, so a scoped
    // credential must be permitted to manage it.
    crate::auth::require_scope(state, user, a.proposer_agent_id).await?;
    // Confirm target exists.
    let _t = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        a.target_agent_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Message safety (when on): the proposal is a note to the target.
    let proposer_message = chakramcp_shared::system_one::note_or_refuse(
        state.compliance.as_deref(),
        a.proposer_message,
        chakramcp_shared::system_one::NoteAbout {
            kind: "friendship_message",
            ..Default::default()
        },
    )
    .await?;
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO friendships
            (id, proposer_agent_id, target_agent_id, status, proposer_message,
             proposer_user_id)
        VALUES ($1, $2, $3, 'proposed', $4, $5)
        "#,
        id,
        a.proposer_agent_id,
        a.target_agent_id,
        proposer_message,
        user.user_id,
    )
    .execute(db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::Conflict("a proposal between these agents is already in flight".into())
        }
        other => other.into(),
    })?;

    crate::events::record_audit(
        db,
        user,
        Some(proposer_account),
        "friendship.propose",
        "friendship",
        Some(id),
        Some(a.target_agent_id),
        "proposed a friendship",
        json!({}),
    )
    .await;

    Ok(json!({ "friendship_id": id, "status": "proposed" }))
}

/// Slug rules mirrored from `handlers::agents::is_valid_slug` (kept
/// private there). 3-32 chars of [a-z0-9-], no leading/trailing or
/// double hyphens.
fn mcp_slug_ok(s: &str) -> bool {
    let len = s.chars().count();
    (3..=32).contains(&len)
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

async fn create_agent(state: &RelayState, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    let db = &state.db;
    #[derive(Deserialize)]
    struct A {
        account_id: Uuid,
        slug: String,
        display_name: String,
        description: Option<String>,
        visibility: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad create_agent args: {e}")))?;

    let slug = a.slug.trim().to_string();
    if !mcp_slug_ok(&slug) {
        return Err(ApiError::InvalidRequest(
            "slug must be 3-32 chars of [a-z0-9-], no leading/trailing or double hyphens".into(),
        ));
    }
    let display_name = a.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(ApiError::InvalidRequest("display_name is required".into()));
    }
    if !user_is_member(db, user.user_id, a.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Resolve visibility: validate if supplied, else fall back to the
    // account's default (matches POST /v1/agents).
    let visibility = match a.visibility.as_deref() {
        Some(v) => {
            if !matches!(v, "private" | "org" | "network") {
                return Err(ApiError::InvalidRequest(
                    "visibility must be private|org|network".into(),
                ));
            }
            v.to_string()
        }
        None => {
            sqlx::query_scalar!(
                r#"SELECT default_agent_visibility FROM accounts WHERE id = $1"#,
                a.account_id
            )
            .fetch_one(db)
            .await?
        }
    };

    // The directory rules, as REST create applies them (spec §5.6–5.7).
    let description = a.description.unwrap_or_default(); // column is NOT NULL DEFAULT ''
    chakramcp_shared::agent_rules::check_description(&visibility, &description)
        .map_err(ApiError::InvalidRequest)?;
    let tags =
        chakramcp_shared::agent_rules::normalize_tags(&a.tags).map_err(ApiError::InvalidRequest)?;

    // Pull mode only over MCP — laptop / sandboxed agents serve the
    // relay inbox and have no public A2A endpoint.
    let id = Uuid::now_v7();
    // Creator attribution, as REST create does (migration 0026), so an
    // `own`-scoped client can act as the agent it just made.
    let created_by_client_id = crate::auth::caller_client_id(db, user.minted_jti).await?;
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agents
            (id, account_id, created_by_user_id, slug, display_name, description,
             visibility, mode, created_by_client_id, created_by_api_key_id, tags)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pull', $8, $9, $10)
        ON CONFLICT (account_id, slug) WHERE tombstoned_at IS NULL DO NOTHING
        RETURNING id, account_id, slug, display_name, description, visibility, tags, created_at
        "#,
        id,
        a.account_id,
        user.user_id,
        slug,
        display_name,
        description,
        visibility,
        created_by_client_id,
        user.api_key_id,
        &tags,
    )
    .fetch_optional(db)
    .await?
    .ok_or_else(|| {
        ApiError::Conflict(format!(
            "agent slug '{slug}' already exists in this account"
        ))
    })?;

    // A `selected` credential keeps managing what it creates (migration 0027).
    let grant = crate::auth::resolve_grant(db, user).await?;
    if grant.mode == crate::auth::AgentScopeMode::Selected {
        if let Some(scope_id) = grant.scope_id {
            sqlx::query!(
                r#"INSERT INTO credential_scope_agents (credential_scope_id, agent_id)
                   VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
                scope_id,
                inserted.id,
            )
            .execute(db)
            .await?;
        }
    }
    state.scope_cache.forget(crate::auth::credential(user));

    crate::events::record_audit(
        db,
        user,
        Some(inserted.account_id),
        "agent.create",
        "agent",
        Some(inserted.id),
        None,
        &format!("registered agent {}", inserted.slug),
        json!({ "slug": inserted.slug, "visibility": inserted.visibility }),
    )
    .await;

    Ok(json!({
        "id": inserted.id,
        "account_id": inserted.account_id,
        "slug": inserted.slug,
        "display_name": inserted.display_name,
        "description": inserted.description,
        "visibility": inserted.visibility,
        "tags": inserted.tags,
        "created_at": inserted.created_at,
    }))
}

async fn publish_capability(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let db = &state.db;
    #[derive(Deserialize)]
    struct A {
        agent_id: Uuid,
        name: String,
        description: Option<String>,
        input_schema: Option<Value>,
        output_schema: Option<Value>,
        visibility: Option<String>,
        semantics: Option<String>,
        #[serde(default)]
        consent: Option<crate::consent::settings::ConsentInput>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad publish_capability args: {e}")))?;

    // Ownership: the caller must be a member of the agent's account.
    let agent = sqlx::query!(
        r#"SELECT account_id, mode FROM agents WHERE id = $1"#,
        a.agent_id
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let agent_account = agent.account_id;
    if !user_is_member(db, user.user_id, agent_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Scope gate (migration 0027): the REST twin (capabilities::create)
    // enforces this, so the MCP path must too — otherwise a scoped
    // credential could publish a capability on an agent it isn't permitted
    // to manage by going through /mcp instead of the REST endpoint.
    crate::auth::require_scope(state, user, a.agent_id).await?;

    let name = a.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(ApiError::InvalidRequest(
            "name must be ascii alphanumeric, underscore, or dot".into(),
        ));
    }
    let visibility = a.visibility.as_deref().unwrap_or("network");
    if !matches!(visibility, "private" | "org" | "network") {
        return Err(ApiError::InvalidRequest(
            "visibility must be private|org|network".into(),
        ));
    }
    let semantics = a.semantics.as_deref().unwrap_or("autonomous");
    if !matches!(semantics, "autonomous" | "human_in_loop") {
        return Err(ApiError::InvalidRequest(
            "semantics must be autonomous|human_in_loop".into(),
        ));
    }
    let input_schema = a
        .input_schema
        .unwrap_or_else(|| json!({ "type": "object" }));
    let output_schema = a
        .output_schema
        .unwrap_or_else(|| json!({ "type": "object" }));
    // Same settings and checks as REST publish (owner consent §3).
    let consent = crate::consent::settings::ConsentSettings::default()
        .apply(db, agent_account, &a.consent.unwrap_or_default())
        .await?;
    crate::consent::settings::refuse_on_push(&consent, &agent.mode)?;

    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agent_capabilities
            (id, agent_id, name, description, input_schema, output_schema,
             visibility, semantics, created_by_user_id,
             consent_required, consent_approvers, consent_max_validity_secs,
             consent_wait_secs)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (agent_id, name) DO NOTHING
        RETURNING id, agent_id, name, description, visibility, created_at
        "#,
        id,
        a.agent_id,
        name,
        a.description.unwrap_or_default(), // column is NOT NULL DEFAULT ''
        input_schema,
        output_schema,
        visibility,
        semantics,
        user.user_id,
        consent.required,
        &consent.approvers,
        consent.max_validity_secs,
        consent.wait_secs,
    )
    .fetch_optional(db)
    .await?
    .ok_or_else(|| {
        ApiError::Conflict(format!("capability '{name}' already exists for this agent"))
    })?;

    crate::events::record_audit(
        db,
        user,
        Some(agent_account),
        "capability.create",
        "capability",
        Some(inserted.id),
        Some(a.agent_id),
        &format!("published capability {}", inserted.name),
        json!({ "name": inserted.name, "visibility": inserted.visibility }),
    )
    .await;

    Ok(json!({
        "id": inserted.id,
        "agent_id": inserted.agent_id,
        "name": inserted.name,
        "description": inserted.description,
        "visibility": inserted.visibility,
        "semantics": semantics,
        "consent": consent.to_dto(true),
        "created_at": inserted.created_at,
    }))
}

// ─── REST-handler bridges ────────────────────────────────
//
// These tools reuse the v1 REST handlers verbatim (same validation,
// auth, and SQL) so the MCP surface stays at parity with the CLI/SDKs
// without duplicating logic. We just translate the flat MCP `arguments`
// object into the handlers' extractor types and serialize their DTO
// back into the MCP result envelope.

use crate::handlers::{agents, capabilities, friendships, grants, reviews};

/// Pull a required UUID field out of the MCP `arguments` object.
fn arg_uuid(args: &Value, key: &str) -> Result<Uuid, ApiError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| ApiError::InvalidRequest(format!("`{key}` (uuid) is required")))
}

/// Deserialize the `arguments` object into a handler request/query type.
/// Unknown fields (e.g. the id we also read via `arg_uuid`) are ignored.
fn from_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, ApiError> {
    serde_json::from_value(args).map_err(|e| ApiError::InvalidRequest(format!("bad args: {e}")))
}

/// Map a REST handler's `Result<Json<Dto>, ApiError>` into the MCP
/// `Result<Value, ApiError>` the dispatcher expects.
fn dto_value<T: Serialize>(r: Result<Json<T>, ApiError>) -> Result<Value, ApiError> {
    r.map(|Json(v)| serde_json::to_value(v).unwrap_or(Value::Null))
}

/// Friendship accept vs reject share a signature; this selects which.
enum Fr {
    Accept,
    Reject,
}

async fn bridge_get_agent(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = match args.get("agent").and_then(Value::as_str) {
        Some(_) => {
            let reference = crate::mcp::discovery::agent_ref_arg(&args, "agent")?;
            crate::discovery::profile::resolve(state, Some(user.user_id), &reference)
                .await?
                .ok_or(ApiError::NotFound)?
                .summary
                .id
        }
        None => arg_uuid(&args, "agent_id")?,
    };
    dto_value(agents::get_one(State(state.clone()), user.clone(), Path(id)).await)
}

async fn bridge_update_agent(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "agent_id")?;
    let req: agents::UpdateRequest = from_args(args)?;
    dto_value(agents::update(State(state.clone()), user.clone(), Path(id), Json(req)).await)
}

async fn bridge_delete_agent(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "agent_id")?;
    agents::delete(State(state.clone()), user.clone(), Path(id))
        .await
        .map(|_| json!({ "deleted": true }))
}

async fn bridge_list_capabilities(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let agent_id = arg_uuid(&args, "agent_id")?;
    dto_value(capabilities::list(State(state.clone()), user.clone(), Path(agent_id)).await)
}

async fn bridge_delete_capability(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let agent_id = arg_uuid(&args, "agent_id")?;
    let cap_id = arg_uuid(&args, "capability_id")?;
    capabilities::delete(State(state.clone()), user.clone(), Path((agent_id, cap_id)))
        .await
        .map(|_| json!({ "deleted": true }))
}

async fn bridge_friendship_response(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
    which: Fr,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "friendship_id")?;
    let req: friendships::ResponseRequest = from_args(args)?;
    let r = match which {
        Fr::Accept => {
            friendships::accept(State(state.clone()), user.clone(), Path(id), Json(req)).await
        }
        Fr::Reject => {
            friendships::reject(State(state.clone()), user.clone(), Path(id), Json(req)).await
        }
    };
    dto_value(r)
}

async fn bridge_counter_friendship(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "friendship_id")?;
    let req: friendships::CounterRequest = from_args(args)?;
    dto_value(friendships::counter(State(state.clone()), user.clone(), Path(id), Json(req)).await)
}

async fn bridge_cancel_friendship(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "friendship_id")?;
    dto_value(friendships::cancel(State(state.clone()), user.clone(), Path(id)).await)
}

async fn bridge_create_grant(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let req: grants::CreateRequest = from_args(args)?;
    dto_value(grants::create(State(state.clone()), user.clone(), Json(req)).await)
}

async fn bridge_revoke_grant(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let id = arg_uuid(&args, "grant_id")?;
    let req: grants::RevokeRequest = from_args(args)?;
    dto_value(grants::revoke(State(state.clone()), user.clone(), Path(id), Json(req)).await)
}

async fn bridge_list_reviews(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let target = arg_uuid(&args, "target_agent_id")?;
    let q: reviews::ListQuery = from_args(args)?;
    dto_value(reviews::list(State(state.clone()), user.clone(), Path(target), Query(q)).await)
}

async fn bridge_write_review(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let target = arg_uuid(&args, "target_agent_id")?;
    let req: reviews::WriteReviewRequest = from_args(args)?;
    dto_value(reviews::write(State(state.clone()), user.clone(), Path(target), Json(req)).await)
}

async fn bridge_review_eligibility(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
) -> Result<Value, ApiError> {
    let target = arg_uuid(&args, "target_agent_id")?;
    dto_value(reviews::eligibility(State(state.clone()), user.clone(), Path(target)).await)
}

async fn bridge_hide_review(
    state: &RelayState,
    user: &AuthUser,
    args: Value,
    hide: bool,
) -> Result<Value, ApiError> {
    let target = arg_uuid(&args, "target_agent_id")?;
    let review_id = arg_uuid(&args, "review_id")?;
    let path = Path((target, review_id));
    let r = if hide {
        reviews::hide(State(state.clone()), user.clone(), path).await
    } else {
        reviews::unhide(State(state.clone()), user.clone(), path).await
    };
    dto_value(r)
}

// ─── /.well-known/oauth-protected-resource ───────────────
//
// Tells MCP clients where to discover the auth server.
pub async fn protected_resource_metadata(State(state): State<RelayState>) -> Json<Value> {
    Json(json!({
        "resource": format!("{}/mcp", state.config.relay_base_url),
        "authorization_servers": [state.config.app_base_url],
        "scopes_supported": ["relay.full"],
        "bearer_methods_supported": ["header"],
    }))
}

#[cfg(test)]
mod manage_agents_tests {
    //! Coverage for the agent-management MCP tools (`create_agent`,
    //! `publish_capability`) added so an agent can self-register over
    //! /mcp instead of requiring out-of-band CLI setup. We drive the
    //! real /mcp JSON-RPC endpoint end-to-end with a Bearer JWT.
    use axum::body::Body;
    use axum::http::{header, Request};
    use chakramcp_shared::config::SharedConfig;
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    async fn seed_user_with_jwt(pool: &PgPool) -> (Uuid, Uuid, String) {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Test User', 'x')"#,
            user_id,
            format!("{user_id}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Acct', 'individual', $3)"#,
            account_id,
            format!("acct-{account_id}"),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        let token = jwt::encode_jwt(
            &jwt::UserClaims::new(user_id, format!("{user_id}@t.local"), false, 1),
            "test-secret-test-secret-test-secret-test-secret",
        )
        .unwrap();
        (user_id, account_id, token)
    }

    /// MCP (via the shared trusted path) refuses an out-of-credits caller with
    /// a client-side back-off error rather than a JSON-RPC 429.
    #[test]
    fn usage_limit_refusals_map_to_a_client_side_rpc_error() {
        use chakramcp_shared::error::ApiError;
        let next_grant_at = chrono::DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let managed = crate::mcp::rpc::api_err_to_rpc(ApiError::InsufficientCredits {
            retry_after_secs: 600,
            next_grant_at,
            credits_url: Some("https://chakramcp.com/app/credits".into()),
        });
        assert_eq!(managed.code, super::ERR_INVALID_REQUEST);
        assert_eq!(
            managed.message,
            "insufficient credits; the free grant arrives on 2026-11-01, \
             or buy credits at https://chakramcp.com/app/credits"
        );
        let data = managed.data.unwrap();
        assert_eq!(data["code"], "account_credits_exhausted");
        assert_eq!(data["retry_after_secs"], 600);
        assert_eq!(data["credits_url"], "https://chakramcp.com/app/credits");

        let self_hosted = crate::mcp::rpc::api_err_to_rpc(ApiError::InsufficientCredits {
            retry_after_secs: 600,
            next_grant_at,
            credits_url: None,
        });
        assert_eq!(
            self_hosted.message,
            "insufficient credits; the free grant arrives on 2026-11-01"
        );
        assert!(self_hosted.data.unwrap().get("credits_url").is_none());

        let rate = crate::mcp::rpc::api_err_to_rpc(ApiError::RateLimited {
            limit: 60,
            retry_after_secs: 23,
        });
        assert_eq!(rate.code, super::ERR_INVALID_REQUEST);
        assert_eq!(rate.message, "rate limit exceeded; retry in 23 s");
        assert_eq!(rate.data.unwrap()["retry_after_secs"], 23);
    }

    /// State whose usage recorder has a live background writer, so tests can
    /// assert on `usage_events` after a `flush`.
    fn recording_state(pool: &PgPool) -> crate::state::RelayState {
        let state = crate::state::RelayState::new(pool.clone(), config());
        let usage = crate::events::UsageRecorder::spawn(state.clone());
        state.with_usage_recorder(usage)
    }

    /// POST a tools/call to /mcp and return the parsed JSON-RPC `result`.
    async fn call(pool: &PgPool, token: &str, name: &str, arguments: Value) -> Value {
        call_on(recording_state(pool), token, name, arguments).await
    }

    /// [`call`] against a given relay state.
    async fn call_on(
        state: crate::state::RelayState,
        token: &str,
        name: &str,
        arguments: Value,
    ) -> Value {
        let app = crate::router(state.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "tools/call",
                            "params": { "name": name, "arguments": arguments },
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "http {}", res.status());
        let body: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        // Usage is written off the request path; wait for it so callers can
        // assert on `usage_events`.
        state.usage.flush().await;
        body["result"].clone()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rest_usage_is_attributed_off_the_request_path(pool: PgPool) {
        let (uid, _account_id, token) = seed_user_with_jwt(&pool).await;
        let state = recording_state(&pool);
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/audit")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "http {}", res.status());
        state.usage.flush().await;

        // The middleware queued only the raw header; the writer resolved it.
        let actor = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT actor_user_id FROM usage_events WHERE surface = 'rest' AND route = '/v1/audit'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(actor, Some(uid));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn create_agent_then_publish_capability(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;

        // 1. create_agent → returns a fresh id, persisted in pull mode.
        let result = call(
            &pool,
            &token,
            "create_agent",
            json!({
                "account_id": account_id,
                "slug": "self-reg-bot",
                "display_name": "Self Reg Bot",
                "description": "Registered over MCP for a test.",
                "visibility": "network",
            }),
        )
        .await;
        assert_eq!(result["isError"], json!(false), "result: {result}");
        let agent_id: Uuid = result["structuredContent"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let row = sqlx::query!(
            "SELECT slug, visibility, mode FROM agents WHERE id = $1",
            agent_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.slug, "self-reg-bot");
        assert_eq!(row.visibility, "network");
        assert_eq!(row.mode, "pull");

        // 2. publish_capability on that agent.
        let result = call(
            &pool,
            &token,
            "publish_capability",
            json!({
                "agent_id": agent_id,
                "name": "negotiate_dinner",
                "description": "negotiate cuisine + drink",
                "input_schema": { "type": "object" },
                "output_schema": { "type": "object" },
            }),
        )
        .await;
        assert_eq!(result["isError"], json!(false), "result: {result}");

        let cap = sqlx::query!(
            "SELECT name, visibility, semantics FROM agent_capabilities WHERE agent_id = $1",
            agent_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cap.name, "negotiate_dinner");
        assert_eq!(cap.visibility, "network"); // defaulted
        assert_eq!(cap.semantics, "autonomous"); // defaulted
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn create_agent_rejects_bad_slug(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let result = call(
            &pool,
            &token,
            "create_agent",
            json!({ "account_id": account_id, "slug": "Bad Slug!", "display_name": "X" }),
        )
        .await;
        // Tool errors surface as isError=true (not protocol errors).
        assert_eq!(result["isError"], json!(true), "result: {result}");
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM agents WHERE account_id = $1",
            account_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, Some(0));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn create_agent_forbidden_for_non_member(pool: PgPool) {
        // Account belongs to user A; user B (different JWT) may not create.
        let (_a, account_id, _token_a) = seed_user_with_jwt(&pool).await;
        let (_b, _acct_b, token_b) = seed_user_with_jwt(&pool).await;
        let result = call(
            &pool,
            &token_b,
            "create_agent",
            json!({ "account_id": account_id, "slug": "intruder-bot", "display_name": "X" }),
        )
        .await;
        assert_eq!(result["isError"], json!(true), "result: {result}");
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM agents WHERE account_id = $1",
            account_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, Some(0));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn create_agent_duplicate_slug_conflicts(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let args = json!({ "account_id": account_id, "slug": "dupe-bot", "display_name": "X" });
        let first = call(&pool, &token, "create_agent", args.clone()).await;
        assert_eq!(first["isError"], json!(false), "first: {first}");
        let second = call(&pool, &token, "create_agent", args).await;
        assert_eq!(second["isError"], json!(true), "result: {second}");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn publish_capability_forbidden_on_foreign_agent(pool: PgPool) {
        // Agent owned by A; B tries to publish a capability on it.
        let (_a, account_id, token_a) = seed_user_with_jwt(&pool).await;
        let (_b, _acct_b, token_b) = seed_user_with_jwt(&pool).await;
        let made = call(
            &pool,
            &token_a,
            "create_agent",
            json!({ "account_id": account_id, "slug": "owned-bot", "display_name": "X" }),
        )
        .await;
        let agent_id = made["structuredContent"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let result = call(
            &pool,
            &token_b,
            "publish_capability",
            json!({ "agent_id": agent_id, "name": "sneaky" }),
        )
        .await;
        assert_eq!(result["isError"], json!(true), "result: {result}");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn writes_record_audit_and_usage_events(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;

        // A write tool: create_agent → audit row + usage row.
        let made = call(
            &pool,
            &token,
            "create_agent",
            json!({ "account_id": account_id, "slug": "evt-bot", "display_name": "E", "visibility": "network", "description": "An agent for testing things." }),
        )
        .await;
        assert_eq!(made["isError"], json!(false), "{made}");

        // A read tool: list_my_agents → usage row, but NO audit row.
        call(&pool, &token, "list_my_agents", json!({})).await;

        let audit_actions: Vec<String> =
            sqlx::query_scalar!("SELECT action FROM audit_events ORDER BY created_at")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(
            audit_actions.contains(&"agent.create".to_string()),
            "expected agent.create audit, got {audit_actions:?}"
        );
        // Reads must NOT be audited.
        assert!(
            !audit_actions.iter().any(|a| a.contains("list")),
            "reads should not be audited: {audit_actions:?}"
        );

        let usage_actions: Vec<String> =
            sqlx::query_scalar!("SELECT action FROM usage_events ORDER BY created_at")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(
            usage_actions.contains(&"mcp:create_agent".to_string()),
            "expected mcp:create_agent usage, got {usage_actions:?}"
        );
        assert!(
            usage_actions.contains(&"mcp:list_my_agents".to_string()),
            "expected mcp:list_my_agents usage (reads ARE metered): {usage_actions:?}"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn audit_and_usage_read_endpoints(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        call(
            &pool,
            &token,
            "create_agent",
            json!({ "account_id": account_id, "slug": "rd-bot", "display_name": "R", "visibility": "network", "description": "An agent for testing things." }),
        )
        .await;
        call(&pool, &token, "list_my_agents", json!({})).await;

        async fn get_json(pool: &PgPool, token: &str, path: &str) -> Value {
            let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
            let res = app
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(path)
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(res.status().is_success(), "http {}", res.status());
            let b = res.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<Value>(&b).unwrap()
        }

        let audit = get_json(&pool, &token, "/v1/audit").await;
        let actions: Vec<&str> = audit["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].as_str().unwrap())
            .collect();
        assert!(actions.contains(&"agent.create"), "audit: {actions:?}");

        let usage = get_json(&pool, &token, "/v1/usage/events").await;
        let totals: Vec<&str> = usage["totals_by_action"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["action"].as_str().unwrap())
            .collect();
        assert!(totals.contains(&"mcp:create_agent"), "usage: {totals:?}");
        assert!(usage["total"].as_i64().unwrap() >= 2);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn list_my_accounts_returns_membership(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let result = call(&pool, &token, "list_my_accounts", json!({})).await;
        // Spec compliance: array-returning tools MUST omit structuredContent
        // (it's typed as an object; strict clients reject a top-level array).
        assert!(
            result.get("structuredContent").is_none(),
            "array tool must not set structuredContent: {result}"
        );
        let body = payload(&result);
        let accounts = body.as_array().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["id"].as_str().unwrap(), account_id.to_string());
        assert_eq!(accounts[0]["role"], json!("owner"));
    }

    /// Decode a non-error tool result the way a spec-compliant client does:
    /// prefer `structuredContent` (object tools), else parse the JSON in the
    /// first `content` text block (array tools omit `structuredContent`).
    fn payload(result: &Value) -> Value {
        assert_eq!(result["isError"], json!(false), "tool errored: {result}");
        if let Some(sc) = result.get("structuredContent") {
            return sc.clone();
        }
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    /// Extract `structuredContent` from a non-error object-returning tool.
    fn ok(result: &Value) -> &Value {
        assert_eq!(result["isError"], json!(false), "tool errored: {result}");
        &result["structuredContent"]
    }

    /// End-to-end: an agent drives the WHOLE social loop over /mcp alone —
    /// register two agents, publish a capability, propose + accept a
    /// friendship, grant access, invoke, claim the inbox, and respond.
    /// This is the parity that lets the voice demo run without CLI/web.
    #[sqlx::test(migrations = "../migrations")]
    async fn full_agent_flow_over_mcp(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let mk = |slug: &str, name: &str| json!({ "account_id": account_id, "slug": slug, "display_name": name, "visibility": "network", "description": "An agent for testing things." });

        // Two agents under the same owner (A = caller, B = capability owner).
        let a = ok(&call(&pool, &token, "create_agent", mk("agent-a", "A")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let b = ok(&call(&pool, &token, "create_agent", mk("agent-b", "B")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();

        // B publishes negotiate_dinner.
        let cap = ok(&call(
            &pool,
            &token,
            "publish_capability",
            json!({ "agent_id": b, "name": "negotiate_dinner" }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();

        // list_capabilities sees it.
        let caps = call(&pool, &token, "list_capabilities", json!({ "agent_id": b })).await;
        assert_eq!(payload(&caps).as_array().unwrap().len(), 1);

        // A proposes friendship to B; B accepts.
        let fr = ok(&call(
            &pool,
            &token,
            "propose_friendship",
            json!({ "proposer_agent_id": a, "target_agent_id": b }),
        )
        .await)["friendship_id"]
            .as_str()
            .unwrap()
            .to_string();
        let accepted = call(
            &pool,
            &token,
            "accept_friendship",
            json!({ "friendship_id": fr }),
        )
        .await;
        assert_eq!(accepted["isError"], json!(false), "accept: {accepted}");
        assert_eq!(accepted["structuredContent"]["status"], json!("accepted"));

        // B grants A access to negotiate_dinner.
        let grant = ok(&call(
            &pool,
            &token,
            "create_grant",
            json!({ "granter_agent_id": b, "grantee_agent_id": a, "capability_id": cap }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();

        // A invokes; B claims via inbox; B responds.
        let inv = ok(&call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "round": 1 } }),
        )
        .await)["invocation_id"]
            .as_str()
            .unwrap()
            .to_string();

        let pulled = call(&pool, &token, "pull_inbox", json!({ "agent_id": b })).await;
        assert_eq!(payload(&pulled).as_array().unwrap().len(), 1);

        let responded = call(
            &pool,
            &token,
            "respond",
            json!({ "invocation_id": inv, "status": "succeeded", "output": { "cuisine": "thai" } }),
        )
        .await;
        assert_eq!(responded["isError"], json!(false), "respond: {responded}");

        // poll confirms terminal state.
        let polled = call(
            &pool,
            &token,
            "poll_invocation",
            json!({ "invocation_id": inv }),
        )
        .await;
        assert_eq!(polled["structuredContent"]["status"], json!("succeeded"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn respond_by_the_approver_in_person_is_confirmed(pool: PgPool) {
        // A human_in_loop capability (message_owner): this caller is the
        // owner in their own session (a plain user JWT, not an agent's
        // token), so their reply is confirmed at once (owner consent §5).
        // Agents' replies become drafts; see consent::tests.
        let m = crate::telemetry::testing::Recorded::start();
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let mk = |slug: &str| json!({ "account_id": account_id, "slug": slug, "display_name": slug, "visibility": "network", "description": "An agent for testing things." });
        let a = ok(&call(&pool, &token, "create_agent", mk("hitl-a")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let b = ok(&call(&pool, &token, "create_agent", mk("hitl-b")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        // B publishes message_owner as human_in_loop.
        let cap = ok(&call(
            &pool,
            &token,
            "publish_capability",
            json!({ "agent_id": b, "name": "message_owner", "semantics": "human_in_loop" }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        // friendship + grant + invoke.
        let fr = ok(&call(
            &pool,
            &token,
            "propose_friendship",
            json!({ "proposer_agent_id": a, "target_agent_id": b }),
        )
        .await)["friendship_id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &pool,
            &token,
            "accept_friendship",
            json!({ "friendship_id": fr }),
        )
        .await;
        let grant = ok(&call(
            &pool,
            &token,
            "create_grant",
            json!({ "granter_agent_id": b, "grantee_agent_id": a, "capability_id": cap }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let inv = ok(&call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "text": "dinner?" } }),
        )
        .await)["invocation_id"]
            .as_str()
            .unwrap()
            .to_string();
        call(&pool, &token, "pull_inbox", json!({ "agent_id": b })).await;

        // The owner answers in person: confirmed and released at once.
        let good = call(&pool, &token, "respond",
            json!({ "invocation_id": inv, "status": "succeeded", "output": { "reply": "yes 8pm" } })).await;
        assert_eq!(
            good["isError"],
            json!(false),
            "an approver's own reply should be confirmed: {good}"
        );
        assert_eq!(ok(&good)["status"], "succeeded");
        let confirmed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM consent_requests
                             WHERE invocation_id = $1::uuid AND kind = 'reply' AND status = 'approved')",
        )
        .bind(&inv)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(confirmed);

        use chakramcp_shared::telemetry::names;
        let calls = names::MCP_TOOL_CALLS_TOTAL;
        assert_eq!(
            m.counter(calls, &[("tool", "respond"), ("result", "ok")]),
            1
        );
        let succeeded = [("mode", "pull"), ("status", "succeeded")];
        assert_eq!(m.counter(names::INVOCATIONS_TOTAL, &succeeded), 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn unknown_tools_are_counted_as_unknown(pool: PgPool) {
        let m = crate::telemetry::testing::Recorded::start();
        let (_uid, _account_id, token) = seed_user_with_jwt(&pool).await;
        call(&pool, &token, "definitely_not_a_tool", json!({})).await;
        call(&pool, &token, "another_made_up_name", json!({})).await;

        let calls = chakramcp_shared::telemetry::names::MCP_TOOL_CALLS_TOTAL;
        let unknown = [
            ("tool", crate::telemetry::UNKNOWN_TOOL),
            ("result", "error"),
        ];
        assert_eq!(m.counter(calls, &unknown), 2);
        let invented = [("tool", "definitely_not_a_tool"), ("result", "error")];
        assert_eq!(
            m.counter(calls, &invented),
            0,
            "raw names never become labels"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn list_invocations_shows_claimed_and_outbound(pool: PgPool) {
        // list_invocations is read-only and, unlike pull_inbox, must keep
        // returning a row after it's been claimed (in_progress) and must
        // surface calls the caller issued (direction=outbound).
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let mk = |slug: &str| json!({ "account_id": account_id, "slug": slug, "display_name": slug, "visibility": "network", "description": "An agent for testing things." });
        let a = ok(&call(&pool, &token, "create_agent", mk("li-a")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let b = ok(&call(&pool, &token, "create_agent", mk("li-b")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let cap = ok(&call(
            &pool,
            &token,
            "publish_capability",
            json!({ "agent_id": b, "name": "negotiate_dinner" }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let fr = ok(&call(
            &pool,
            &token,
            "propose_friendship",
            json!({ "proposer_agent_id": a, "target_agent_id": b }),
        )
        .await)["friendship_id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &pool,
            &token,
            "accept_friendship",
            json!({ "friendship_id": fr }),
        )
        .await;
        let grant = ok(&call(
            &pool,
            &token,
            "create_grant",
            json!({ "granter_agent_id": b, "grantee_agent_id": a, "capability_id": cap }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "text": "dinner?" } }),
        )
        .await;

        // Inbound for B: one pending row before claiming.
        let pending = payload(
            &call(
                &pool,
                &token,
                "list_invocations",
                json!({ "direction": "inbound", "agent_id": b, "status": "pending" }),
            )
            .await,
        );
        assert_eq!(pending.as_array().unwrap().len(), 1);

        // Claim it → in_progress. pull_inbox is self-deduping…
        call(&pool, &token, "pull_inbox", json!({ "agent_id": b })).await;
        let pulled_again =
            payload(&call(&pool, &token, "pull_inbox", json!({ "agent_id": b })).await);
        assert_eq!(
            pulled_again.as_array().unwrap().len(),
            0,
            "pull_inbox must NOT re-return a claimed row"
        );

        // …but list_invocations still shows the claimed-but-unanswered row.
        let in_prog = payload(
            &call(
                &pool,
                &token,
                "list_invocations",
                json!({ "direction": "inbound", "agent_id": b, "status": "in_progress" }),
            )
            .await,
        );
        assert_eq!(
            in_prog.as_array().unwrap().len(),
            1,
            "list_invocations must surface claimed in_progress work"
        );

        // Outbound view from A (the caller) sees the same call.
        let outbound = payload(
            &call(
                &pool,
                &token,
                "list_invocations",
                json!({ "direction": "outbound", "agent_id": a }),
            )
            .await,
        );
        assert_eq!(outbound.as_array().unwrap().len(), 1);
        assert_eq!(outbound[0]["grantee_agent_id"], json!(a));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn create_grant_requires_friendship(pool: PgPool) {
        // No friendship → grant must be rejected.
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let mk = |slug: &str| json!({ "account_id": account_id, "slug": slug, "display_name": slug, "visibility": "network", "description": "An agent for testing things." });
        let a = ok(&call(&pool, &token, "create_agent", mk("grant-a")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let b = ok(&call(&pool, &token, "create_agent", mk("grant-b")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let cap = ok(&call(
            &pool,
            &token,
            "publish_capability",
            json!({ "agent_id": b, "name": "x" }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let grant = call(
            &pool,
            &token,
            "create_grant",
            json!({ "granter_agent_id": b, "grantee_agent_id": a, "capability_id": cap }),
        )
        .await;
        assert_eq!(grant["isError"], json!(true), "grant: {grant}");
    }

    /// Two agents under one owner, a capability on B, an accepted
    /// friendship, and a grant from B to A — all over /mcp. Returns
    /// (token, grantee A, grant id, friendship id).
    async fn seed_granted_pair(pool: &PgPool) -> (String, String, String, String) {
        let (_uid, account_id, token) = seed_user_with_jwt(pool).await;
        let mk = |slug: &str| json!({ "account_id": account_id, "slug": slug, "display_name": slug, "visibility": "network", "description": "An agent for testing things." });
        let a = ok(&call(pool, &token, "create_agent", mk("inv-a")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let b = ok(&call(pool, &token, "create_agent", mk("inv-b")).await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let cap = ok(&call(
            pool,
            &token,
            "publish_capability",
            json!({ "agent_id": b, "name": "x" }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        let fr = ok(&call(
            pool,
            &token,
            "propose_friendship",
            json!({ "proposer_agent_id": a, "target_agent_id": b }),
        )
        .await)["friendship_id"]
            .as_str()
            .unwrap()
            .to_string();
        ok(&call(
            pool,
            &token,
            "accept_friendship",
            json!({ "friendship_id": fr }),
        )
        .await);
        let grant = ok(&call(
            pool,
            &token,
            "create_grant",
            json!({ "granter_agent_id": b, "grantee_agent_id": a, "capability_id": cap }),
        )
        .await)["id"]
            .as_str()
            .unwrap()
            .to_string();
        (token, a, grant, fr)
    }

    async fn invocation_row(pool: &PgPool, grant: &str) -> (String, Option<String>, Value, Value) {
        let grant: Uuid = grant.parse().unwrap();
        sqlx::query_as::<_, (String, Option<String>, Value, Option<Value>)>(
            "SELECT status, error_message, input_preview, trust_snapshot
               FROM relay_invocations WHERE grant_id = $1",
        )
        .bind(grant)
        .fetch_one(pool)
        .await
        .map(|(st, err, input, snap)| (st, err, input, snap.unwrap_or(Value::Null)))
        .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_rejected_without_accepted_friendship(pool: PgPool) {
        let (token, a, grant, fr) = seed_granted_pair(&pool).await;
        // Accepted friendships have no API exit today; simulate the
        // relationship ending underneath a still-active grant.
        sqlx::query("UPDATE friendships SET status = 'cancelled' WHERE id = $1")
            .bind(fr.parse::<Uuid>().unwrap())
            .execute(&pool)
            .await
            .unwrap();

        let res = call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "q": 1 } }),
        )
        .await;
        assert_eq!(res["isError"], json!(true), "invoke: {res}");

        let (status, err, _, _) = invocation_row(&pool, &grant).await;
        assert_eq!(status, "rejected");
        assert!(err.unwrap().contains("friendship"));
    }

    async fn agent_account(pool: &PgPool, agent: &str) -> Uuid {
        sqlx::query_scalar("SELECT account_id FROM agents WHERE id = $1")
            .bind(agent.parse::<Uuid>().unwrap())
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_queues_one_charge(pool: PgPool) {
        let (token, a, grant, _) = seed_granted_pair(&pool).await;
        let inv: Uuid = ok(&call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "q": 1 } }),
        )
        .await)["invocation_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        // Charged to the invoking (grantee) agent's account.
        let queued: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT invocation_id, account_id FROM credit_charge_queue")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(queued, vec![(inv, agent_account(&pool, &a).await)]);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_flags_a_low_balance(pool: PgPool) {
        let (token, a, grant, _) = seed_granted_pair(&pool).await;
        let cache = crate::limits::CreditCache::default();
        cache.replace_all(
            std::collections::HashSet::new(),
            std::collections::HashSet::from([agent_account(&pool, &a).await]),
            std::collections::HashMap::new(),
        );
        let state = recording_state(&pool)
            .with_limits_enforce(true)
            .with_credit_cache(std::sync::Arc::new(cache));
        let res = call_on(
            state,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "q": 1 } }),
        )
        .await;
        assert_eq!(res["isError"], json!(false), "invoke: {res}");
        assert_eq!(res["_meta"]["chakramcp/credits"], json!("low"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_refused_when_out_of_credits(pool: PgPool) {
        let (token, a, grant, _) = seed_granted_pair(&pool).await;
        let credits =
            crate::limits::credits::exhausted_cache(&pool, agent_account(&pool, &a).await).await;
        let state = recording_state(&pool)
            .with_limits_enforce(true)
            .with_credit_cache(credits);

        let res = call_on(
            state,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "q": 1 } }),
        )
        .await;
        assert_eq!(res["isError"], json!(true), "invoke: {res}");
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with("Error: insufficient credits; the free grant arrives on "),
            "{text}"
        );
        assert!(
            !text.contains("buy credits"),
            "no buy link when self-hosted"
        );
        // The same facts as fields, for clients that read them.
        assert_eq!(
            res["structuredContent"]["code"],
            json!("account_credits_exhausted")
        );
        assert!(
            res["structuredContent"]["retry_after_secs"]
                .as_u64()
                .unwrap()
                >= 1
        );
        let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_charge_queue")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(queued, 0, "refused before anything was queued");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_truncates_large_input(pool: PgPool) {
        let (token, a, grant, _) = seed_granted_pair(&pool).await;
        let big = "x".repeat(20 * 1024);
        ok(&call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "blob": big } }),
        )
        .await);

        let (status, _, input, _) = invocation_row(&pool, &grant).await;
        assert_eq!(status, "pending");
        assert_eq!(
            input["__chakramcp_truncated__"],
            json!(true),
            "input: {input}"
        );
        assert!(input["original_byte_length"].as_u64().unwrap() > 20 * 1024);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_records_trust_snapshot(pool: PgPool) {
        let (token, a, grant, fr) = seed_granted_pair(&pool).await;
        ok(&call(
            &pool,
            &token,
            "invoke",
            json!({ "grant_id": grant, "grantee_agent_id": a, "input": { "q": 1 } }),
        )
        .await);

        let (_, _, input, snap) = invocation_row(&pool, &grant).await;
        assert_eq!(input, json!({ "q": 1 }));
        assert_eq!(snap["grant"]["id"], json!(grant), "snapshot: {snap}");
        assert_eq!(snap["grant"]["status"], json!("active"));
        assert_eq!(snap["grant"]["capability_name"], json!("x"));
        assert_eq!(snap["friendship"]["id"], json!(fr));
        assert_eq!(snap["friendship"]["status"], json!("accepted"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn create_grant_accepts_and_validates_purpose(pool: PgPool) {
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let mk = |slug: &str| json!({ "account_id": account_id, "slug": slug, "display_name": slug, "visibility": "network", "description": "An agent for testing things." });
        let a = ok(&call(&pool, &token, "create_agent", mk("p-a")).await)["id"].clone();
        let b = ok(&call(&pool, &token, "create_agent", mk("p-b")).await)["id"].clone();
        let cap = ok(&call(
            &pool,
            &token,
            "publish_capability",
            json!({ "agent_id": b, "name": "x" }),
        )
        .await)["id"]
            .clone();
        let fr = ok(&call(
            &pool,
            &token,
            "propose_friendship",
            json!({ "proposer_agent_id": a, "target_agent_id": b }),
        )
        .await)["friendship_id"]
            .clone();
        ok(&call(
            &pool,
            &token,
            "accept_friendship",
            json!({ "friendship_id": fr }),
        )
        .await);
        let grant = |purpose: Value| json!({ "granter_agent_id": b, "grantee_agent_id": a, "capability_id": cap, "purpose": purpose });

        let too_long = call(&pool, &token, "create_grant", grant(json!("x".repeat(501)))).await;
        assert_eq!(too_long["isError"], json!(true), "{too_long}");

        let created = call(
            &pool,
            &token,
            "create_grant",
            grant(json!("  Book team dinners  ")),
        )
        .await;
        assert_eq!(ok(&created)["purpose"], json!("Book team dinners"));
    }

    #[test]
    fn new_tools_are_advertised() {
        let listed = super::tools_list_result(false, false);
        let names: Vec<&str> = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in [
            "list_my_accounts",
            "create_agent",
            "publish_capability",
            "get_agent",
            "update_agent",
            "delete_agent",
            "list_capabilities",
            "delete_capability",
            "accept_friendship",
            "reject_friendship",
            "counter_friendship",
            "cancel_friendship",
            "create_grant",
            "revoke_grant",
            "list_reviews",
            "write_review",
            "review_eligibility",
            "hide_review",
            "unhide_review",
        ] {
            assert!(names.contains(&expected), "missing tool: {expected}");
        }
    }
}

#[cfg(test)]
mod server_info_tests {
    //! The MCP handshake reports the release this build is, like `--version`
    //! and `chakramcp_build_info`, not the relay crate's own version.

    #[test]
    fn initialize_reports_the_release_version() {
        let release = option_env!("CHAKRAMCP_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
        assert_eq!(super::initialize_result()["serverInfo"]["version"], release);
    }
}
