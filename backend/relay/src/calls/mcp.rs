//! The calls tools on `/mcp` (spec §6.2), listed only when calls are on.
//! Each takes the same arguments as its REST route and calls the same code.

use chakramcp_shared::error::ApiError;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{feed, lifecycle, memory, messages, read};
use crate::auth::AuthUser;
use crate::mcp::rpc::tool;
use crate::state::RelayState;

const UUID: &str = "uuid";

fn id(description: &str) -> Value {
    json!({ "type": "string", "format": UUID, "description": description })
}

/// The tools' definitions, for `tools/list`.
pub fn tools() -> Vec<Value> {
    let agent = id("One of your agents: the party acting.");
    let call = id("The call.");
    vec![
        tool(
            "start_call",
            "Invite another agent to a call: a two-way conversation that carries messages and \
             requests to each other's capabilities. The callee's agent accepts or declines. \
             memory 'call' asks the relay to keep the call's memory; 'own' (default) leaves it \
             to each agent. ttl_secs is how long the content is kept after the call ends.",
            json!({
                "type": "object",
                "required": ["from_agent_id", "to_agent_id"],
                "properties": {
                    "from_agent_id": agent,
                    "to_agent_id": id("The agent to call."),
                    "memory": { "type": "string", "enum": ["own", "call"] },
                    "ttl_secs": { "type": "integer", "minimum": 0 },
                    "idle_secs": { "type": "integer", "minimum": 3600, "maximum": 604800 },
                    "topic": { "type": "string", "maxLength": 200 },
                    "message": { "type": "string", "description": "An opening message." }
                }
            }),
        ),
        tool(
            "list_calls",
            "List calls of your agents, newest first.",
            json!({
                "type": "object",
                "properties": {
                    "agent_id": id("Only this agent's calls."),
                    "state": { "type": "string", "enum": ["ringing", "active", "declined", "missed", "ended"] },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "cursor": { "type": "string" }
                }
            }),
        ),
        tool(
            "get_call",
            "Read one call: its parties, terms and state.",
            json!({ "type": "object", "required": ["call_id"], "properties": { "call_id": call } }),
        ),
        tool(
            "answer_call",
            "Accept or decline a call that's ringing for your agent. Accepting may carry a \
             first reply; declining may carry a reason.",
            json!({
                "type": "object",
                "required": ["call_id", "agent_id", "answer"],
                "properties": {
                    "call_id": call,
                    "agent_id": agent,
                    "answer": { "type": "string", "enum": ["accept", "decline"] },
                    "message": { "type": "string" },
                    "reason": { "type": "string", "maxLength": 500 }
                }
            }),
        ),
        tool(
            "send_message",
            "Send a message on an active call. Messages are screened: contact details shared to \
             move off ChakraMCP are removed, and harmful ones refused.",
            json!({
                "type": "object",
                "required": ["call_id", "agent_id", "text"],
                "properties": {
                    "call_id": call,
                    "agent_id": agent,
                    "text": { "type": "string" },
                    "data": { "type": "object" }
                }
            }),
        ),
        tool(
            "pull_calls",
            "Claim your agent's new call events, oldest first, each once: invites, messages, \
             answers and ends.",
            json!({
                "type": "object",
                "required": ["agent_id"],
                "properties": {
                    "agent_id": agent,
                    "limit": { "type": "integer", "minimum": 1, "maximum": 25 }
                }
            }),
        ),
        tool(
            "call_history",
            "Read a call's transcript, oldest first, or search it (q matches any of the words).",
            json!({
                "type": "object",
                "required": ["call_id"],
                "properties": {
                    "call_id": call,
                    "after": { "type": "integer", "description": "Entries after this seq." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "q": { "type": "string" }
                }
            }),
        ),
        tool(
            "call_memory",
            "Read a call's memory, when the caller asked the relay to keep it: pinned \
             decisions, commitments and facts, open questions, and the latest messages, \
             all verbatim.",
            json!({ "type": "object", "required": ["call_id"], "properties": { "call_id": call } }),
        ),
        tool(
            "ask_call_memory",
            "Ask a call's memory a question ('what date did we agree?'). Returns the messages \
             that answer it, verbatim and best first, and how sure it is that any does.",
            json!({
                "type": "object",
                "required": ["call_id", "agent_id", "question"],
                "properties": {
                    "call_id": call,
                    "agent_id": agent,
                    "question": { "type": "string", "maxLength": 1024 }
                }
            }),
        ),
        tool(
            "end_call",
            "Hang up a call (or withdraw an invite). The other agent is told.",
            json!({
                "type": "object",
                "required": ["call_id", "agent_id"],
                "properties": { "call_id": call, "agent_id": agent }
            }),
        ),
    ]
}

/// Whether `name` is a calls tool.
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "start_call"
            | "list_calls"
            | "get_call"
            | "answer_call"
            | "send_message"
            | "pull_calls"
            | "call_history"
            | "call_memory"
            | "ask_call_memory"
            | "end_call"
    )
}

fn args<T: DeserializeOwned>(tool: &str, value: Value) -> Result<T, ApiError> {
    let value = if value.is_null() { json!({}) } else { value };
    serde_json::from_value(value)
        .map_err(|e| ApiError::InvalidRequest(format!("bad {tool} args: {e}")))
}

/// Run a calls tool.
pub async fn call(
    state: &RelayState,
    user: &AuthUser,
    name: &str,
    arguments: Value,
) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct CallId {
        call_id: Uuid,
    }
    #[derive(Deserialize)]
    struct Answer {
        call_id: Uuid,
        agent_id: Uuid,
        answer: String,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    }
    #[derive(Deserialize)]
    struct Send {
        call_id: Uuid,
        #[serde(flatten)]
        req: messages::SendRequest,
    }
    #[derive(Deserialize)]
    struct History {
        call_id: Uuid,
        #[serde(flatten)]
        q: messages::HistoryQuery,
    }
    #[derive(Deserialize)]
    struct Ask {
        call_id: Uuid,
        #[serde(flatten)]
        req: memory::recall::AskRequest,
    }
    #[derive(Deserialize)]
    struct End {
        call_id: Uuid,
        #[serde(flatten)]
        req: lifecycle::EndRequest,
    }

    match name {
        "start_call" => Ok(lifecycle::invite(state, user, args(name, arguments)?)
            .await?
            .dto()),
        "list_calls" => read::list(state, user, args(name, arguments)?).await,
        "get_call" => {
            let a: CallId = args(name, arguments)?;
            read::get(state, user, a.call_id).await
        }
        "answer_call" => {
            let a: Answer = args(name, arguments)?;
            let call = match a.answer.as_str() {
                "accept" => {
                    lifecycle::accept(
                        state,
                        user,
                        a.call_id,
                        lifecycle::AcceptRequest {
                            agent_id: a.agent_id,
                            message: a.message,
                        },
                    )
                    .await?
                }
                "decline" => {
                    lifecycle::decline(
                        state,
                        user,
                        a.call_id,
                        lifecycle::DeclineRequest {
                            agent_id: a.agent_id,
                            reason: a.reason,
                        },
                    )
                    .await?
                }
                _ => {
                    return Err(ApiError::InvalidRequest(
                        "answer must be accept or decline".into(),
                    ))
                }
            };
            Ok(call.dto())
        }
        "send_message" => {
            let a: Send = args(name, arguments)?;
            Ok(messages::send(state, user, a.call_id, a.req).await?.dto())
        }
        "pull_calls" => Ok(json!(
            feed::claim(state, user, args(name, arguments)?).await?
        )),
        "call_history" => {
            let a: History = args(name, arguments)?;
            Ok(json!(messages::history(state, user, a.call_id, a.q).await?))
        }
        "call_memory" => {
            let a: CallId = args(name, arguments)?;
            memory::get(state, user, a.call_id).await
        }
        "ask_call_memory" => {
            let a: Ask = args(name, arguments)?;
            memory::recall::ask(state, user, a.call_id, a.req).await
        }
        "end_call" => {
            let a: End = args(name, arguments)?;
            Ok(lifecycle::end(state, user, a.call_id, a.req).await?.dto())
        }
        other => Err(ApiError::InvalidRequest(format!("unknown tool '{other}'"))),
    }
}
