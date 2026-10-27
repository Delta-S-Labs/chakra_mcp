//! `POST /mcp/directory`: the public agent directory over MCP, with no
//! sign-in (MCP and discovery spec §6.3). Search, profiles, featured agents
//! and reviews of network-visible agents; nothing can be called from here.
//!
//! It never answers 401, so a client never starts a sign-in. Each client IP
//! gets `MCP_DIRECTORY_PER_IP_PER_MIN` requests a minute through the shared
//! rate limiter. Requests are anonymous, so they're counted in a metric, not
//! written to `usage_events`. Off (404) unless `MCP_AGENTS_ENABLED` and
//! `DISCOVERY_V2`.

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::discovery;
use super::rpc::{self, rpc_err, RpcRequest, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND};
use crate::state::RelayState;

const SERVER_NAME: &str = "chakramcp-directory";
/// The largest request body read.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// JSON-RPC's code for a body that isn't JSON.
const ERR_PARSE: i32 = -32700;

const INSTRUCTIONS: &str = "The ChakraMCP agent directory: find agents and read what they \
     offer, without signing in. search_agents finds them (any word matches, best first), \
     get_agent_profile shows one agent's capabilities with their input schemas, reviews and \
     how to call it, featured_agents lists good ones to try first, and list_reviews shows \
     what callers said. This endpoint can't call anything: to call an agent, connect \
     https://<relay>/mcp, or the agent's own endpoint /agents/<account>/<agent>/mcp, and \
     sign in.";

/// The rate limiter's key for a client IP: a UUID from the SHA-256 of the
/// address, so it can't collide with an account id. Unknown IPs share one.
fn ip_key(ip: Option<IpAddr>) -> Uuid {
    let seed = ip.map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
    let digest = Sha256::digest(format!("mcp-directory:{seed}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

pub async fn handle(State(state): State<RelayState>, req: Request) -> Response {
    if !(state.mcp.agents_enabled && state.config.discovery_v2_enabled) {
        return ApiError::NotFound.into_response();
    }
    let ip = super::client_ip(&req, state.mcp.trust_forwarded_for);
    let outcome = state
        .rate_limiter
        .check(ip_key(ip), state.mcp.directory_per_ip_per_min)
        .await;
    crate::telemetry::record_directory_request(outcome.is_limited());
    if outcome.is_limited() {
        let retry_after = outcome.info().map_or(60, |info| info.reset_secs);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry_after.to_string())],
            Json(json!({
                "error": {
                    "code": "directory_rate_limited",
                    "message": format!(
                        "this directory takes {} requests a minute from one address; retry in {retry_after} s",
                        state.mcp.directory_per_ip_per_min
                    ),
                    "retryable": true,
                }
            })),
        )
            .into_response();
    }

    let body = match axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return rpc::reply_err(Value::Null, ERR_INVALID_REQUEST, "request too large"),
    };
    let req: RpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => return rpc::reply_err(Value::Null, ERR_PARSE, format!("not JSON-RPC: {e}")),
    };
    let id = req.id.clone().unwrap_or(Value::Null);
    if req.jsonrpc != "2.0" {
        return rpc::reply_err(id, ERR_INVALID_REQUEST, "jsonrpc must be '2.0'");
    }
    let result = match req.method.as_str() {
        "initialize" => Ok(rpc::initialize(SERVER_NAME, INSTRUCTIONS)),
        "notifications/initialized" | "notifications/cancelled" => {
            return StatusCode::NO_CONTENT.into_response();
        }
        "tools/list" => Ok(json!({
            "tools": [
                discovery::search_tool(false),
                discovery::profile_tool(false),
                discovery::featured_tool(),
                discovery::reviews_tool(),
            ]
        })),
        "tools/call" => call_tool(&state, req.params).await,
        "ping" => Ok(json!({})),
        other => Err(rpc_err(
            ERR_METHOD_NOT_FOUND,
            format!("method '{other}' not found"),
        )),
    };
    if req.id.is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }
    rpc::reply(id, result)
}

async fn call_tool(state: &RelayState, params: Value) -> Result<Value, rpc::RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let result = match name {
        "search_agents" => {
            if args.get("callable").and_then(Value::as_bool) == Some(true) {
                Err(ApiError::InvalidRequest(
                    "callable needs a signed-in caller: connect /mcp instead".into(),
                ))
            } else {
                discovery::search_agents(state, None, args).await
            }
        }
        "get_agent_profile" => discovery::get_agent_profile(state, None, &args).await,
        "featured_agents" => discovery::featured_agents(state, None).await,
        "list_reviews" => discovery::list_reviews(state, None, &args).await,
        other => {
            return Err(rpc_err(
                rpc::ERR_INVALID_PARAMS,
                format!("unknown tool '{other}'"),
            ))
        }
    };
    let (envelope, ok) = rpc::tool_result(result);
    crate::telemetry::record_tool_call(name, ok);
    Ok(envelope)
}
