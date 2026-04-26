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

use axum::extract::State;
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
        _ => Err(rpc_err(ERR_METHOD_NOT_FOUND, format!("method '{}' not found", req.method))),
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
        error: Some(RpcError { code, message: message.into(), data: None }),
    })
    .into_response()
}

fn rpc_err(code: i32, message: impl Into<String>) -> RpcError {
    RpcError { code, message: message.into(), data: None }
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
    RpcError { code, message: msg, data: None }
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
            "ChakraMCP relay. Use list_my_agents to see your agents, list_grants \
             to see what you can call, invoke + poll_invocation to call a friend's \
             capability, and pull_inbox + respond to serve requests directed at \
             your own agents."
    })
}

// ─── tools/list ──────────────────────────────────────────

fn tools_list_result() -> Value {
    json!({
        "tools": [
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

    let result = match p.name.as_str() {
        "list_my_agents" => list_my_agents(&state.db, user).await,
        "list_network_agents" => list_network_agents(&state.db, user).await,
        "list_grants" => list_grants(&state.db, user, p.arguments).await,
        "list_friendships" => list_friendships(&state.db, user, p.arguments).await,
        "invoke" => invoke(&state.db, user, p.arguments).await,
        "poll_invocation" => poll_invocation(&state.db, user, p.arguments).await,
        "pull_inbox" => pull_inbox(&state.db, user, p.arguments).await,
        "respond" => respond(&state.db, user, p.arguments).await,
        "propose_friendship" => propose_friendship(&state.db, user, p.arguments).await,
        other => return Err(rpc_err(ERR_INVALID_PARAMS, format!("unknown tool '{other}'"))),
    };

    match result {
        Ok(value) => Ok(json!({
            "content": [
                { "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }
            ],
            "structuredContent": value,
            "isError": false,
        })),
        Err(api_err) => {
            // Per MCP, tool errors come back as a successful response
            // with isError=true so the LLM can read them, *not* as a
            // protocol-level RPC error.
            let rpc = api_err_to_rpc(api_err);
            Ok(json!({
                "content": [
                    { "type": "text", "text": format!("Error: {}", rpc.message) }
                ],
                "isError": true,
            }))
        }
    }
}

// ─── Tool implementations ────────────────────────────────

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

    Ok(json!(rows.into_iter().map(|r| json!({
        "id": r.id,
        "account_id": r.account_id,
        "account_slug": r.account_slug,
        "account_display_name": r.account_display_name,
        "slug": r.slug,
        "display_name": r.display_name,
        "description": r.description,
        "visibility": r.visibility,
        "capability_count": r.capability_count,
    })).collect::<Vec<_>>()))
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

    Ok(json!(rows.into_iter().map(|r| json!({
        "id": r.id,
        "account_id": r.account_id,
        "account_slug": r.account_slug,
        "account_display_name": r.account_display_name,
        "slug": r.slug,
        "display_name": r.display_name,
        "description": r.description,
        "capability_count": r.capability_count,
        "is_mine": r.is_mine,
    })).collect::<Vec<_>>()))
}

async fn list_grants(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize, Default)]
    struct A { direction: Option<String> }
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

    Ok(json!(rows.into_iter().map(|r| json!({
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
    })).collect::<Vec<_>>()))
}

async fn list_friendships(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize, Default)]
    struct A { direction: Option<String>, status: Option<String> }
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

    Ok(json!(rows.into_iter().map(|r| json!({
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
    })).collect::<Vec<_>>()))
}

async fn invoke(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A { grant_id: Uuid, grantee_agent_id: Uuid, input: Value }
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
    .fetch_optional(db).await?.ok_or(ApiError::NotFound)?;

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
            "grant is {}; only active grants can be invoked", row.grant_status
        )));
    }
    if let Some(exp) = row.expires_at {
        if exp <= Utc::now() {
            return Err(ApiError::Conflict("grant has expired".into()));
        }
    }

    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, input_preview)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
        "#,
        id, row.grant_id, row.granter_agent_id, row.grantee_agent_id,
        row.capability_id, row.capability_name, user.user_id, a.input,
    ).execute(db).await?;

    Ok(json!({ "invocation_id": id, "status": "pending" }))
}

async fn poll_invocation(db: &PgPool, user: &AuthUser, args: Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A { invocation_id: Uuid }
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
        user.user_id, a.invocation_id,
    ).fetch_optional(db).await?.ok_or(ApiError::NotFound)?;

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
    struct A { agent_id: Uuid, limit: Option<i64> }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad pull_inbox args: {e}")))?;
    let limit = a.limit.unwrap_or(25).clamp(1, 100);

    let agent_account = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#, a.agent_id,
    ).fetch_optional(db).await?.ok_or(ApiError::NotFound)?;
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
        a.agent_id, limit, user.user_id,
    ).fetch_all(&mut *tx).await?;
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
    ).fetch_all(db).await?;

    Ok(json!(rows.into_iter().map(|r| json!({
        "invocation_id": r.id,
        "capability_name": r.capability_name,
        "grantee_agent_id": r.grantee_agent_id,
        "grantee_display_name": r.grantee_display_name,
        "input": r.input_preview,
        "created_at": r.created_at,
        "claimed_at": r.claimed_at,
    })).collect::<Vec<_>>()))
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
    ).fetch_optional(db).await?.ok_or(ApiError::NotFound)?;

    if row.status != "in_progress" {
        return Err(ApiError::Conflict(format!(
            "invocation is {}; only in_progress can be completed", row.status
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
        a.invocation_id, a.status, elapsed_ms,
        a.error.as_deref(), a.output.unwrap_or(Value::Null),
    ).execute(db).await?;

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
        r#"SELECT account_id FROM agents WHERE id = $1"#, a.proposer_agent_id,
    ).fetch_optional(db).await?.ok_or(ApiError::NotFound)?;
    if !user_is_member(db, user.user_id, proposer_account).await? {
        return Err(ApiError::Forbidden);
    }
    // Confirm target exists.
    let _t = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#, a.target_agent_id,
    ).fetch_optional(db).await?.ok_or(ApiError::NotFound)?;

    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status, proposer_message)
        VALUES ($1, $2, $3, 'proposed', $4)
        "#,
        id, a.proposer_agent_id, a.target_agent_id, a.proposer_message,
    )
    .execute(db).await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::Conflict("a proposal between these agents is already in flight".into())
        }
        other => other.into(),
    })?;

    Ok(json!({ "friendship_id": id, "status": "proposed" }))
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
