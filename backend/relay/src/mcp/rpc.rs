//! JSON-RPC 2.0 and MCP plumbing shared by the relay's MCP endpoints (MCP
//! and discovery spec §6.7): `/mcp`, `/mcp/directory` and agents' own
//! endpoints. The envelope, error codes, mapping API errors onto them, tool
//! definitions and results, and `initialize`.

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use chakramcp_shared::error::ApiError;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// The release this build reports (e.g. `0.2.0`, or `edge` from main).
pub const SERVER_VERSION: &str = chakramcp_shared::telemetry::VERSION;

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
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub const ERR_INVALID_REQUEST: i32 = -32600;
pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERR_INVALID_PARAMS: i32 = -32602;
pub const ERR_INTERNAL: i32 = -32603;

pub fn reply_err(id: Value, code: i32, message: impl Into<String>) -> Response {
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

pub fn rpc_err(code: i32, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}

pub fn api_err_to_rpc(e: ApiError) -> RpcError {
    use chakramcp_shared::error::ApiError::*;
    let (code, msg) = match &e {
        InvalidRequest(m) => (ERR_INVALID_PARAMS, m.clone()),
        Unauthorized => (ERR_INVALID_REQUEST, "unauthorized".into()),
        Forbidden => (ERR_INVALID_REQUEST, "forbidden".into()),
        NotFound => (ERR_INVALID_REQUEST, "not found".into()),
        Conflict(m) => (ERR_INVALID_REQUEST, m.clone()),
        // Usage limits: a client-side "back off" error (no JSON-RPC 429). The
        // message says when to retry; `data` carries the same as fields.
        RateLimited {
            limit,
            retry_after_secs,
        } => {
            return RpcError {
                code: ERR_INVALID_REQUEST,
                message: format!("{e}; retry in {retry_after_secs} s"),
                data: Some(json!({
                    "code": "account_rate_limited",
                    "limit": limit,
                    "retry_after_secs": retry_after_secs,
                })),
            };
        }
        InsufficientCredits {
            retry_after_secs,
            next_grant_at,
            credits_url,
        } => {
            let when = next_grant_at.format("%Y-%m-%d");
            let message = match credits_url {
                Some(url) => {
                    format!("{e}; the free grant arrives on {when}, or buy credits at {url}")
                }
                None => format!("{e}; the free grant arrives on {when}"),
            };
            let mut data = json!({
                "code": "account_credits_exhausted",
                "retry_after_secs": retry_after_secs,
                "next_grant_at": next_grant_at,
            });
            if let Some(url) = credits_url {
                data["credits_url"] = json!(url);
            }
            return RpcError {
                code: ERR_INVALID_REQUEST,
                message,
                data: Some(data),
            };
        }
        // The app's sign-up, sign-in and checkout errors; the relay never
        // returns them.
        SignupDisabled | SigninRateLimited { .. } | TooManyCheckouts => {
            (ERR_INVALID_REQUEST, e.to_string())
        }
        PaymentProvider => (ERR_INTERNAL, e.to_string()),
        PayloadTooLarge { code, message } => {
            return RpcError {
                code: ERR_INVALID_PARAMS,
                message: message.clone(),
                data: Some(json!({ "code": code })),
            };
        }
        Policy { code, message } | Coded { code, message, .. } => {
            return RpcError {
                code: ERR_INVALID_REQUEST,
                message: message.clone(),
                data: Some(json!({ "code": code })),
            };
        }
        Database(_) | Auth(_) | Internal(_) => (ERR_INTERNAL, e.to_string()),
    };
    RpcError {
        code,
        message: msg,
        data: None,
    }
}

/// A JSON-RPC response with `result` or `error`.
pub fn reply(id: Value, result: Result<Value, RpcError>) -> Response {
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(e) => (None, Some(e)),
    };
    Json(RpcResponse {
        jsonrpc: "2.0",
        id,
        result,
        error,
    })
    .into_response()
}

/// `initialize`'s result for a server called `name`.
pub fn initialize(name: &str, instructions: &str) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": name,
            "version": SERVER_VERSION,
        },
        "instructions": instructions,
    })
}

pub fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

/// A tool's result envelope, and whether it succeeded.
///
/// The full JSON always travels in `content` text. A JSON object is also
/// `structuredContent`; the MCP spec types `structuredContent` as an object,
/// and strict clients (e.g. the OpenAI Agents SDK's pydantic CallToolResult)
/// reject a top-level array, so list tools' arrays travel in `content` only.
///
/// Per MCP, a tool's error is a successful response with `isError` so the
/// model can read it, not a protocol-level error. A refusal's code and
/// fields (retry hint, next grant, where to buy) are its
/// `structuredContent`.
pub fn tool_result(result: Result<Value, ApiError>) -> (Value, bool) {
    match result {
        Ok(value) => {
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
            let rpc = api_err_to_rpc(api_err);
            let mut envelope = json!({
                "content": [
                    { "type": "text", "text": format!("Error: {}", rpc.message) }
                ],
                "isError": true,
            });
            if let Some(data) = rpc.data {
                envelope["structuredContent"] = data;
            }
            (envelope, false)
        }
    }
}
