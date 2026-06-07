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
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "chakramcp-relay";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ─── JSON-RPC envelope ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

const ERR_INVALID_REQUEST: i32 = -32600;
const ERR_METHOD_NOT_FOUND: i32 = -32601;
const ERR_INVALID_PARAMS: i32 = -32602;
const ERR_INTERNAL: i32 = -32603;

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

    let result = match req.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "notifications/initialized" | "notifications/cancelled" => {
            // Spec: notifications get no response body.
            return StatusCode::NO_CONTENT.into_response();
        }
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => call_tool(&state, &user, req.params).await,
        "ping" => Ok(json!({})),
        _ => Err(rpc_err(
            ERR_METHOD_NOT_FOUND,
            format!("method '{}' not found", req.method),
        )),
    };

    if is_notification {
        return StatusCode::NO_CONTENT.into_response();
    }

    match result {
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
    }
}

fn reply_err(id: Value, code: i32, message: impl Into<String>) -> Response {
    Json(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.into(),
            data: None,
        }),
    })
    .into_response()
}

fn rpc_err(code: i32, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}

fn api_err_to_rpc(e: ApiError) -> RpcError {
    use chakramcp_shared::error::ApiError::*;
    let (code, msg) = match &e {
        InvalidRequest(m) => (ERR_INVALID_PARAMS, m.clone()),
        Unauthorized => (ERR_INVALID_REQUEST, "unauthorized".into()),
        Forbidden => (ERR_INVALID_REQUEST, "forbidden".into()),
        NotFound => (ERR_INVALID_REQUEST, "not found".into()),
        Conflict(m) => (ERR_INVALID_REQUEST, m.clone()),
        Database(_) | Auth(_) | Internal(_) => (ERR_INTERNAL, e.to_string()),
    };
    RpcError {
        code,
        message: msg,
        data: None,
    }
}

// ─── initialize ──────────────────────────────────────────

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION,
        },
        "instructions":
            "ChakraMCP relay. Use create_agent + publish_capability to register \
             yourself and the RPCs you expose. Use list_my_agents to see your \
             agents, list_grants to see what you can call, invoke + poll_invocation \
             to call a friend's capability, and pull_inbox + respond to serve \
             requests directed at your own agents."
    })
}

// ─── tools/list ──────────────────────────────────────────

fn tools_list_result() -> Value {
    json!({
        "tools": [
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
                "Enqueue an invocation against a granted capability. Returns immediately with invocation_id; poll with poll_invocation.",
                json!({
                    "type": "object",
                    "required": ["grant_id", "grantee_agent_id", "input"],
                    "properties": {
                        "grant_id": { "type": "string", "format": "uuid" },
                        "grantee_agent_id": { "type": "string", "format": "uuid", "description": "One of your agents — the caller side." },
                        "input": { "type": "object" }
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
            tool("respond",
                "Post the result for an in_progress invocation you claimed via pull_inbox.",
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
                        "description":  { "type": "string" },
                        "visibility":   { "type": "string", "enum": ["private", "org", "network"], "description": "Defaults to the account's default visibility. Use 'network' to make the agent discoverable by peers." }
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
                        "visibility":    { "type": "string", "enum": ["private", "network"], "description": "Defaults to 'network'." },
                        "semantics":     { "type": "string", "enum": ["autonomous", "human_in_loop"], "description": "Defaults to 'autonomous'." }
                    }
                })),
            tool("get_agent",
                "Fetch one agent by id (yours, or any network-visible agent).",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": { "agent_id": { "type": "string", "format": "uuid" } }
                })),
            tool("update_agent",
                "Patch one of your agents' metadata (display_name, description, visibility, endpoint_url, agent_card_url).",
                json!({
                    "type": "object",
                    "required": ["agent_id"],
                    "properties": {
                        "agent_id":     { "type": "string", "format": "uuid" },
                        "display_name": { "type": "string" },
                        "description":  { "type": "string" },
                        "visibility":   { "type": "string", "enum": ["private", "org", "network"] },
                        "endpoint_url": { "type": "string" },
                        "agent_card_url": { "type": "string" }
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
                        "expires_at":       { "type": "string", "format": "date-time", "description": "Optional RFC3339 expiry." }
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
    })
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

// ─── tools/call dispatch ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn call_tool(state: &RelayState, user: &AuthUser, params: Value) -> Result<Value, RpcError> {
    let p: CallParams = serde_json::from_value(params)
        .map_err(|e| rpc_err(ERR_INVALID_PARAMS, format!("malformed call params: {e}")))?;

    let tool = p.name.clone();
    let result = match p.name.as_str() {
        "list_my_accounts" => list_my_accounts(&state.db, user).await,
        "list_my_agents" => list_my_agents(&state.db, user).await,
        "list_network_agents" => list_network_agents(&state.db, user).await,
        "list_grants" => list_grants(&state.db, user, p.arguments).await,
        "list_friendships" => list_friendships(&state.db, user, p.arguments).await,
        "invoke" => invoke(&state.db, user, p.arguments).await,
        "poll_invocation" => poll_invocation(&state.db, user, p.arguments).await,
        "pull_inbox" => pull_inbox(&state.db, user, p.arguments).await,
        "respond" => respond(&state.db, user, p.arguments).await,
        "propose_friendship" => propose_friendship(&state.db, user, p.arguments).await,
        "create_agent" => create_agent(&state.db, user, p.arguments).await,
        "publish_capability" => publish_capability(&state.db, user, p.arguments).await,
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
        other => {
            return Err(rpc_err(
                ERR_INVALID_PARAMS,
                format!("unknown tool '{other}'"),
            ))
        }
    };

    let (envelope, ok) = match result {
        Ok(value) => {
            // The full JSON always travels in `content` text. We ALSO set
            // `structuredContent`, but only when the value is a JSON object:
            // the MCP spec types `structuredContent` as an object, and strict
            // clients (e.g. the OpenAI Agents SDK's pydantic CallToolResult)
            // reject a top-level array. List tools return arrays, so for those
            // we omit `structuredContent` and clients parse `content` text.
            let mut envelope = json!({
                "content": [
                    { "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }
                ],
                "isError": false,
            });
            if value.is_object() {
                envelope["structuredContent"] = value;
            }
            (envelope, true)
        }
        Err(api_err) => {
            // Per MCP, tool errors come back as a successful response
            // with isError=true so the LLM can read them, *not* as a
            // protocol-level RPC error.
            let rpc = api_err_to_rpc(api_err);
            (
                json!({
                    "content": [
                        { "type": "text", "text": format!("Error: {}", rpc.message) }
                    ],
                    "isError": true,
                }),
                false,
            )
        }
    };

    // Meter every MCP tool call (reads + writes) for billing. Audit for
    // write tools is recorded inside each tool impl / bridged REST handler.
    crate::events::record_usage(
        &state.db,
        Some(user),
        None,
        "mcp",
        &format!("mcp:{tool}"),
        "MCP",
        &tool,
        if ok { 200 } else { 400 },
    )
    .await;

    Ok(envelope)
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
        ORDER BY a.created_at DESC
        "#,
        user.user_id,
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

async fn list_network_agents(db: &PgPool, user: &AuthUser) -> Result<Value, ApiError> {
    let rows = sqlx::query!(
        r#"
        SELECT a.id, a.account_id, a.slug, a.display_name, a.description,
               acc.slug as account_slug, acc.display_name as account_display_name,
               (SELECT COUNT(*)::bigint FROM agent_capabilities c
                  WHERE c.agent_id = a.id AND c.visibility = 'network') as "capability_count!",
               EXISTS(
                   SELECT 1 FROM account_memberships m
                   WHERE m.account_id = a.account_id AND m.user_id = $1
               ) as "is_mine!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.visibility = 'network'
        ORDER BY a.created_at DESC
        "#,
        user.user_id,
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
            "capability_count": r.capability_count,
            "is_mine": r.is_mine,
        }))
        .collect::<Vec<_>>()))
}

async fn list_grants(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
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

async fn list_friendships(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
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

async fn invoke(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        grant_id: Uuid,
        grantee_agent_id: Uuid,
        input: Value,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad invoke args: {e}")))?;

    let row = sqlx::query!(
        r#"
        SELECT g.id as grant_id, g.status as grant_status,
               g.granter_agent_id, g.grantee_agent_id, g.capability_id,
               g.expires_at,
               ea.account_id as grantee_account_id,
               cap.name as capability_name
        FROM grants g
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        WHERE g.id = $1
        "#,
        a.grant_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.grantee_agent_id != a.grantee_agent_id {
        return Err(ApiError::InvalidRequest(
            "grantee_agent_id does not match the grant".into(),
        ));
    }
    if !user_is_member(db, user.user_id, row.grantee_account_id).await? {
        return Err(ApiError::Forbidden);
    }
    if row.grant_status != "active" {
        return Err(ApiError::Conflict(format!(
            "grant is {}; only active grants can be invoked",
            row.grant_status
        )));
    }
    if let Some(exp) = row.expires_at {
        if exp <= Utc::now() {
            return Err(ApiError::Conflict("grant has expired".into()));
        }
    }

    // `api_key_id` follows the same caller-side semantics as
    // POST /v1/invoke (see handlers/invoke.rs): the credential that
    // authed this MCP `invoke` tool call. NULL when the caller used
    // a user JWT.
    //
    // `minted_jti` is the dual: NULL on the ck_ path, set on the JWT
    // path so the per-pair dashboard can attribute the call.
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, input_preview,
             api_key_id, minted_jti)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9, $10)
        "#,
        id,
        row.grant_id,
        row.granter_agent_id,
        row.grantee_agent_id,
        row.capability_id,
        row.capability_name,
        user.user_id,
        a.input,
        user.api_key_id,
        user.minted_jti,
    )
    .execute(db)
    .await?;

    Ok(json!({ "invocation_id": id, "status": "pending" }))
}

async fn poll_invocation(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        invocation_id: Uuid,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad poll args: {e}")))?;

    let r = sqlx::query!(
        r#"
        SELECT i.id, i.status, i.elapsed_ms, i.error_message,
               i.input_preview, i.output_preview,
               i.created_at, i.claimed_at, i.capability_name,
               EXISTS(
                   SELECT 1 FROM account_memberships m
                   JOIN agents a ON a.account_id = m.account_id
                   WHERE m.user_id = $1
                     AND (a.id = i.granter_agent_id OR a.id = i.grantee_agent_id)
               ) as "may_view!"
        FROM relay_invocations i
        WHERE i.id = $2
        "#,
        user.user_id,
        a.invocation_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !r.may_view {
        return Err(ApiError::NotFound);
    }

    Ok(json!({
        "id": r.id,
        "status": r.status,
        "capability_name": r.capability_name,
        "elapsed_ms": r.elapsed_ms,
        "error_message": r.error_message,
        "input_preview": r.input_preview,
        "output_preview": r.output_preview,
        "created_at": r.created_at,
        "claimed_at": r.claimed_at,
    }))
}

async fn pull_inbox(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
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

    let rows = sqlx::query!(
        r#"
        SELECT i.id, i.capability_name, i.input_preview,
               i.created_at, i.claimed_at,
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
        .map(|r| json!({
            "invocation_id": r.id,
            "capability_name": r.capability_name,
            "grantee_agent_id": r.grantee_agent_id,
            "grantee_display_name": r.grantee_display_name,
            "input": r.input_preview,
            "created_at": r.created_at,
            "claimed_at": r.claimed_at,
        }))
        .collect::<Vec<_>>()))
}

async fn respond(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        invocation_id: Uuid,
        status: String,
        output: Option<Value>,
        error: Option<String>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad respond args: {e}")))?;
    if !matches!(a.status.as_str(), "succeeded" | "failed") {
        return Err(ApiError::InvalidRequest(
            "status must be 'succeeded' or 'failed'".into(),
        ));
    }

    let row = sqlx::query!(
        r#"
        SELECT i.status, i.created_at,
               ga.account_id as "granter_account_id?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        WHERE i.id = $1
        "#,
        a.invocation_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "in_progress" {
        return Err(ApiError::Conflict(format!(
            "invocation is {}; only in_progress can be completed",
            row.status
        )));
    }
    let granter_account = row.granter_account_id.ok_or(ApiError::Forbidden)?;
    if !user_is_member(db, user.user_id, granter_account).await? {
        return Err(ApiError::Forbidden);
    }

    let elapsed_ms = (Utc::now() - row.created_at)
        .num_milliseconds()
        .clamp(0, i32::MAX as i64) as i32;

    sqlx::query!(
        r#"
        UPDATE relay_invocations
        SET status = $2, elapsed_ms = $3, error_message = $4, output_preview = $5
        WHERE id = $1 AND status = 'in_progress'
        "#,
        a.invocation_id,
        a.status,
        elapsed_ms,
        a.error.as_deref(),
        a.output.unwrap_or(Value::Null),
    )
    .execute(db)
    .await?;

    Ok(json!({ "invocation_id": a.invocation_id, "status": a.status, "elapsed_ms": elapsed_ms }))
}

async fn propose_friendship(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
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
    // Confirm target exists.
    let _t = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        a.target_agent_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

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
        a.proposer_message,
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

async fn create_agent(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        account_id: Uuid,
        slug: String,
        display_name: String,
        description: Option<String>,
        visibility: Option<String>,
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

    // Pull mode only over MCP — laptop / sandboxed agents serve the
    // relay inbox and have no public A2A endpoint.
    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agents
            (id, account_id, created_by_user_id, slug, display_name, description,
             visibility, mode)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pull')
        ON CONFLICT (account_id, slug) WHERE tombstoned_at IS NULL DO NOTHING
        RETURNING id, account_id, slug, display_name, description, visibility, created_at
        "#,
        id,
        a.account_id,
        user.user_id,
        slug,
        display_name,
        a.description.unwrap_or_default(), // column is NOT NULL DEFAULT ''
        visibility,
    )
    .fetch_optional(db)
    .await?
    .ok_or_else(|| {
        ApiError::Conflict(format!(
            "agent slug '{slug}' already exists in this account"
        ))
    })?;

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
        "created_at": inserted.created_at,
    }))
}

async fn publish_capability(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        agent_id: Uuid,
        name: String,
        description: Option<String>,
        input_schema: Option<Value>,
        output_schema: Option<Value>,
        visibility: Option<String>,
        semantics: Option<String>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad publish_capability args: {e}")))?;

    // Ownership: the caller must be a member of the agent's account.
    let agent_account =
        sqlx::query_scalar!(r#"SELECT account_id FROM agents WHERE id = $1"#, a.agent_id)
            .fetch_optional(db)
            .await?
            .ok_or(ApiError::NotFound)?;
    if !user_is_member(db, user.user_id, agent_account).await? {
        return Err(ApiError::Forbidden);
    }

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
    if !matches!(visibility, "private" | "network") {
        return Err(ApiError::InvalidRequest(
            "visibility must be private|network".into(),
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

    let id = Uuid::now_v7();
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agent_capabilities
            (id, agent_id, name, description, input_schema, output_schema,
             visibility, semantics, created_by_user_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
    let id = arg_uuid(&args, "agent_id")?;
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

    /// POST a tools/call to /mcp and return the parsed JSON-RPC `result`.
    async fn call(pool: &PgPool, token: &str, name: &str, arguments: Value) -> Value {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
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
        body["result"].clone()
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
                "description": "registered over MCP",
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
            json!({ "account_id": account_id, "slug": "evt-bot", "display_name": "E", "visibility": "network" }),
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
            json!({ "account_id": account_id, "slug": "rd-bot", "display_name": "R", "visibility": "network" }),
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
        let mk = |slug: &str, name: &str| json!({ "account_id": account_id, "slug": slug, "display_name": name, "visibility": "network" });

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
    async fn create_grant_requires_friendship(pool: PgPool) {
        // No friendship → grant must be rejected.
        let (_uid, account_id, token) = seed_user_with_jwt(&pool).await;
        let mk = |slug: &str| json!({ "account_id": account_id, "slug": slug, "display_name": slug, "visibility": "network" });
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

    #[test]
    fn new_tools_are_advertised() {
        let listed = super::tools_list_result();
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
