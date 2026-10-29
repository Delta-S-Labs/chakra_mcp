//! `POST /agents/{account_slug}/{agent_slug}/mcp`: one agent as an MCP
//! server (MCP and discovery spec §6.5). Its capabilities you can call now
//! are its tools; a call runs invoke's path as your agent (the caller rules
//! of §6.1, or `?as=`) and waits for the result.
//!
//! At the HTTP level: 401 with this endpoint's own sign-in metadata without
//! a valid token; 404 when the agent is missing, deleted, not visible to
//! you, or the endpoint is off (`MCP_AGENTS_ENABLED`, `DISCOVERY_V2`).
//! Everything else is JSON-RPC, as on `/mcp`, so clients show the message.

use std::collections::HashSet;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::rpc::{self, rpc_err, RpcError, RpcRequest, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND};
use crate::auth::AuthUser;
use crate::discovery::fields::{Access, Loaded};
use crate::discovery::profile::{self, AgentRef};
use crate::discovery::standing;
use crate::handlers::invoke::{Outcome, StartOptions, Target};
use crate::state::RelayState;

const ABOUT: &str = "chakramcp_about";
const GET_RESULT: &str = "chakramcp_get_result";
/// MCP tool names: 1–64 of letters, digits, `_` and `-`.
const MAX_TOOL_NAME: usize = 64;

#[derive(Debug, Default, Deserialize)]
pub struct AgentQuery {
    /// The agent to call as: an id, `account/agent`, or a slug unique among
    /// yours.
    #[serde(rename = "as")]
    pub as_agent: Option<String>,
}

pub async fn handle(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
    Query(query): Query<AgentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !(state.mcp.agents_enabled && state.config.discovery_v2_enabled) {
        return ApiError::NotFound.into_response();
    }
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(user) = crate::auth::authenticate(&state, auth).await else {
        // The `www_authenticate` layer names this endpoint's metadata.
        return ApiError::Unauthorized.into_response();
    };
    let reference = AgentRef::Name {
        account: account_slug.to_lowercase(),
        slug: agent_slug.to_lowercase(),
    };
    let target = match profile::resolve(&state, Some(user.user_id), &reference).await {
        Ok(Some(target)) => target,
        Ok(None) => return ApiError::NotFound.into_response(),
        Err(e) => return e.into_response(),
    };

    let req: RpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => return rpc::reply_err(Value::Null, -32700, format!("not JSON-RPC: {e}")),
    };
    let id = req.id.clone().unwrap_or(Value::Null);
    if req.method.starts_with("notifications/") {
        return StatusCode::NO_CONTENT.into_response();
    }
    if req.jsonrpc != "2.0" {
        return rpc::reply_err(id, rpc::ERR_INVALID_REQUEST, "jsonrpc must be '2.0'");
    }
    let as_agent = match query.as_agent.as_deref() {
        None => None,
        Some(raw) => match resolve_as(&state, &user, raw).await {
            Ok(agent) => Some(agent),
            Err(e) => return rpc::reply(id, Err(rpc::api_err_to_rpc(e))),
        },
    };
    let endpoint = Endpoint {
        state: &state,
        user: &user,
        target: &target,
        as_agent,
    };
    let result = match req.method.as_str() {
        "initialize" => endpoint.initialize(),
        "tools/list" => endpoint.tools_list().await,
        "tools/call" => endpoint.tools_call(req.params).await,
        "ping" => Ok(json!({})),
        other => Err(rpc_err(
            ERR_METHOD_NOT_FOUND,
            format!("method '{other}' not found"),
        )),
    };
    rpc::reply(id, result)
}

/// `?as=`: one of your agents in the credential's scope.
async fn resolve_as(state: &RelayState, user: &AuthUser, raw: &str) -> Result<Uuid, ApiError> {
    let mine = standing::my_agents(state, user).await?;
    let candidates = sqlx::query!(
        r#"
        SELECT a.id, a.slug, acc.slug AS account_slug
          FROM agents a
          JOIN accounts acc ON acc.id = a.account_id
          JOIN account_memberships m ON m.account_id = a.account_id AND m.user_id = $1
         WHERE a.tombstoned_at IS NULL
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    let raw = raw.trim().to_lowercase();
    let found: Vec<Uuid> = candidates
        .iter()
        .filter(|a| {
            a.id.to_string() == raw
                || a.slug == raw
                || format!("{}/{}", a.account_slug, a.slug) == raw
        })
        .map(|a| a.id)
        .collect();
    match found.as_slice() {
        [one] if mine.iter().any(|a| a.id == *one) => Ok(*one),
        [_] => Err(crate::auth::out_of_scope()),
        [] => Err(ApiError::InvalidRequest(format!(
            "as: no agent `{raw}` of yours"
        ))),
        _ => Err(ApiError::InvalidRequest(format!(
            "as: several of your agents are called `{raw}`; name one as account/agent"
        ))),
    }
}

/// A capability offered as a tool.
#[derive(Debug, Clone)]
struct Offered {
    tool: String,
    capability: String,
    description: String,
    input_schema: Value,
    /// The capability's input isn't an object: the tool takes it as `input`.
    wrapped: bool,
    output_schema: Value,
}

struct Endpoint<'a> {
    state: &'a RelayState,
    user: &'a AuthUser,
    target: &'a Loaded,
    as_agent: Option<Uuid>,
}

impl Endpoint<'_> {
    fn name(&self) -> String {
        format!(
            "{}/{}",
            self.target.summary.account_slug, self.target.summary.slug
        )
    }

    fn initialize(&self) -> Result<Value, RpcError> {
        let agent = &self.target.summary;
        if agent.mode == "push" {
            return Err(RpcError {
                code: rpc::ERR_INVALID_REQUEST,
                message: format!(
                    "{} is a push agent with its own A2A endpoint; call it through its card",
                    self.name()
                ),
                data: Some(json!({
                    "code": "chk.mcp.push_agent",
                    "a2a_card": format!(
                        "{}/agents/{}/.well-known/agent-card.json",
                        self.state.config.relay_base_url.trim_end_matches('/'),
                        self.name()
                    ),
                })),
            });
        }
        let caller = match self.as_agent {
            Some(_) => "the agent you named with ?as=".to_owned(),
            None => "your agent holding a grant for the capability, or for a public one \
                     your default agent"
                .to_owned(),
        };
        let instructions = format!(
            "{}\n\nThe tools here are the capabilities of {} ({}) you can call now. Calls are \
             made as {caller}. A result that takes a while comes back as still running: \
             fetch it with {GET_RESULT}. {ABOUT} shows the agent's profile, what else it offers \
             and how to get access.",
            agent.description.trim(),
            agent.display_name,
            self.name()
        );
        Ok(rpc::initialize(&agent.display_name, &instructions))
    }

    /// The capabilities you can call now, as tools, and the names of those
    /// you can't yet.
    async fn offered(&self) -> Result<(Vec<Offered>, Vec<String>), ApiError> {
        let callers: Vec<Uuid> = match self.as_agent {
            Some(id) => vec![id],
            None => standing::my_agents(self.state, self.user)
                .await?
                .into_iter()
                .map(|a| a.id)
                .collect(),
        };
        let granted: HashSet<Uuid> = sqlx::query_scalar!(
            r#"
            SELECT DISTINCT capability_id FROM grants
             WHERE granter_agent_id = $1 AND grantee_agent_id = ANY($2)
               AND status = 'active' AND (expires_at IS NULL OR expires_at > now())
            "#,
            self.target.summary.id,
            &callers,
        )
        .fetch_all(&self.state.db)
        .await?
        .into_iter()
        .collect();
        let caps = sqlx::query!(
            r#"
            SELECT id, name, description, input_schema, output_schema, visibility,
                   public_invoke, consent_required
              FROM agent_capabilities
             WHERE agent_id = $1
             ORDER BY name
            "#,
            self.target.summary.id,
        )
        .fetch_all(&self.state.db)
        .await?;

        let mut taken: HashSet<String> = [ABOUT.to_owned(), GET_RESULT.to_owned()].into();
        let mut offered = Vec::new();
        let mut not_yet = Vec::new();
        for c in caps
            .into_iter()
            .filter(|c| self.target.relation.sees(&c.visibility))
        {
            let public =
                Access::of(c.public_invoke, &self.target.summary.visibility) == Access::Public;
            if !(public || granted.contains(&c.id)) {
                not_yet.push(c.name);
                continue;
            }
            let (input_schema, wrapped) = input_schema(&c.input_schema);
            let mut description = c.description.trim().to_owned();
            if c.consent_required {
                description.push_str(" Asks its owner before running.");
            }
            offered.push(Offered {
                tool: unique_tool_name(&c.name, &mut taken),
                capability: c.name,
                description: description.trim().to_owned(),
                input_schema,
                wrapped,
                output_schema: output_schema(&c.output_schema),
            });
        }
        Ok((offered, not_yet))
    }

    async fn tools_list(&self) -> Result<Value, RpcError> {
        let (offered, _) = self.offered().await.map_err(rpc::api_err_to_rpc)?;
        let mut tools: Vec<Value> = offered
            .iter()
            .map(|o| {
                json!({
                    "name": o.tool,
                    "title": o.capability,
                    "description": o.description,
                    "inputSchema": o.input_schema,
                    "outputSchema": o.output_schema,
                })
            })
            .collect();
        tools.push(rpc::tool(
            ABOUT,
            "This agent's profile: what it does, its owner, all its capabilities and who can call each, reviews, your standing, and how to get access to what you can't call yet.",
            json!({ "type": "object", "properties": {} }),
        ));
        tools.push(rpc::tool(
            GET_RESULT,
            "The result of an earlier call to this agent that was still running: pass its invocation_id. Waits up to wait_seconds for it.",
            json!({
                "type": "object",
                "required": ["invocation_id"],
                "properties": {
                    "invocation_id": { "type": "string", "format": "uuid" },
                    "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 55 }
                }
            }),
        ));
        Ok(json!({ "tools": tools }))
    }

    async fn tools_call(&self, params: Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_err(ERR_INVALID_PARAMS, "tools/call needs a name"))?
            .to_owned();
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let (result, label) = match name.as_str() {
            ABOUT => (rpc::tool_result(self.about().await).0, ABOUT),
            GET_RESULT => (self.get_result(&args).await, GET_RESULT),
            tool => (self.call_capability(tool, args).await, "agent_capability"),
        };
        let ok = result["isError"] != json!(true);
        crate::telemetry::record_tool_call(label, ok);
        self.state.usage.record(crate::events::UsageEvent {
            actor: crate::events::UsageActor::User(self.user.clone()),
            account_id: None,
            surface: "mcp_agent",
            action: format!("mcp_agent:{label}"),
            method: "MCP".to_owned(),
            route: label.to_owned(),
            status_code: if ok { 200 } else { 400 },
        });
        Ok(result)
    }

    /// `chakramcp_about`: the profile with your standing, what you can't
    /// call yet, and how to get access.
    async fn about(&self) -> Result<Value, ApiError> {
        let (_, not_yet) = self.offered().await?;
        let mut out =
            json!(profile::profile(self.state, Some(self.user), self.target.clone()).await?);
        if !not_yet.is_empty() {
            out["not_callable_yet"] = json!(not_yet);
            out["how_to_get_access"] = json!(format!(
                "These need a friendship and a grant. Propose a friendship with {} from one of \
                 your agents (on /mcp: propose_friendship), and once it's accepted ask its owner \
                 to grant the capability.",
                self.name()
            ));
        }
        Ok(out)
    }

    async fn call_capability(&self, tool: &str, args: Value) -> Value {
        let offered = match self.offered().await {
            Ok((offered, _)) => offered,
            Err(e) => return refused(e, None),
        };
        let Some(o) = offered.into_iter().find(|o| o.tool == tool) else {
            return refused(
                ApiError::coded(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    format!(
                        "no tool `{tool}` you can call on {}: list the tools again, or see {ABOUT}",
                        self.name()
                    ),
                ),
                None,
            );
        };
        let input = if o.wrapped {
            args.get("input").cloned().unwrap_or(Value::Null)
        } else {
            args
        };
        let outcome = crate::handlers::invoke::start(
            self.state,
            self.user,
            Target::Named {
                agent: AgentRef::Id(self.target.summary.id),
                capability: o.capability,
            },
            &input,
            StartOptions {
                as_agent: self.as_agent,
                consent_terms: None,
                call: None,
                limits_source: "mcp_agent",
            },
        )
        .await;
        match outcome {
            Err(e) => refused(e, None),
            Ok(Outcome::Enqueued { invocation_id, .. }) => {
                self.result(
                    invocation_id,
                    Duration::from_secs(self.state.mcp.tool_wait_secs),
                )
                .await
            }
            Ok(Outcome::Held { invocation_id, .. }) => {
                self.result(invocation_id, Duration::ZERO).await
            }
            Ok(Outcome::Rejected {
                invocation_id,
                error,
                code,
                safety,
                ..
            }) => refusal(
                code.unwrap_or("rejected"),
                &error,
                Some(invocation_id),
                safety.map(|s| json!(s)),
            ),
            Ok(Outcome::QuotaExhausted { quota, resets_at }) => refusal(
                "monthly_quota_exhausted",
                &crate::handlers::invoke::quota_message(quota, resets_at),
                None,
                None,
            ),
        }
    }

    /// `chakramcp_get_result`: an invocation of this agent made by one of
    /// your agents in scope (404 otherwise), waited for as a tool call is.
    async fn get_result(&self, args: &Value) -> Value {
        let Some(invocation_id) = args
            .get("invocation_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            return refused(
                ApiError::InvalidRequest("invocation_id must be a uuid".into()),
                None,
            );
        };
        let mine = match standing::my_agents(self.state, self.user).await {
            Ok(mine) => mine,
            Err(e) => return refused(e, None),
        };
        let row = sqlx::query!(
            "SELECT granter_agent_id, grantee_agent_id FROM relay_invocations WHERE id = $1",
            invocation_id,
        )
        .fetch_optional(&self.state.db)
        .await;
        let ours = matches!(
            row,
            Ok(Some(ref r)) if r.granter_agent_id == Some(self.target.summary.id)
                && r.grantee_agent_id.is_some_and(|g| mine.iter().any(|a| a.id == g))
        );
        if !ours {
            return refused(ApiError::NotFound, None);
        }
        let wait = args
            .get("wait_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.state.mcp.tool_wait_secs)
            .min(self.state.mcp.tool_wait_secs);
        self.result(invocation_id, Duration::from_secs(wait)).await
    }

    /// Wait up to `wait` for the invocation, then its tool result (spec
    /// §6.5's table).
    async fn result(&self, invocation_id: Uuid, wait: Duration) -> Value {
        if let Err(e) = super::wait::wait_for(self.state, invocation_id, wait).await {
            return refused(e, Some(invocation_id));
        }
        let (view, hint) =
            match crate::handlers::mcp::invocation_view(self.state, self.user, invocation_id).await
            {
                Ok(view) => view,
                Err(e) => return refused(e, Some(invocation_id)),
            };
        // Whether the target is around, while it's open (waking pull agents
        // §8).
        let with_peer = |mut structured: Value| {
            if let Some(peer) = view.get("peer") {
                structured["peer"] = peer.clone();
            }
            structured
        };
        let status = view["status"].as_str().unwrap_or_default();
        let id = invocation_id.to_string();
        match status {
            "succeeded" => {
                let output = view["output_preview"].clone();
                let mut structured =
                    json!({ "status": "succeeded", "invocation_id": id, "output": output });
                if let Some(safety) = view.get("safety") {
                    structured["safety"] = safety.clone();
                }
                json!({
                    "content": [{ "type": "text", "text": serde_json::to_string_pretty(&output).unwrap_or_default() }],
                    "structuredContent": structured,
                    "isError": false,
                })
            }
            "awaiting_consent" => json!({
                "content": [{ "type": "text", "text": format!(
                    "The agent's owner approves this request first. Check back with {GET_RESULT} (invocation_id {id})."
                ) }],
                "structuredContent": with_peer(json!({ "status": "awaiting_consent", "invocation_id": id })),
                "isError": false,
            }),
            "failed" | "timeout" => {
                let error = view["error_message"].as_str().unwrap_or(status).to_owned();
                json!({
                    "content": [{ "type": "text", "text": error }],
                    "structuredContent": { "status": status, "invocation_id": id, "error": error },
                    "isError": true,
                })
            }
            "rejected" => refusal(
                "rejected",
                view["error_message"].as_str().unwrap_or("rejected"),
                Some(invocation_id),
                view.get("safety").cloned(),
            ),
            // pending, in_progress
            other => {
                let mut text = format!("Still running. Call {GET_RESULT} with invocation_id {id}.");
                if let Some(hint) = hint {
                    text = format!("{text} {hint}");
                }
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": with_peer(json!({ "status": other, "invocation_id": id })),
                    "isError": false,
                })
            }
        }
    }
}

/// A refused call's tool result: `structuredContent` matches the tools'
/// output schema (status `refused`, its code), with the invocation id when
/// one was recorded and what message safety found.
fn refusal(code: &str, message: &str, invocation_id: Option<Uuid>, safety: Option<Value>) -> Value {
    let mut structured = json!({ "status": "refused", "code": code, "error": message });
    if let Some(id) = invocation_id {
        structured["invocation_id"] = json!(id.to_string());
    }
    if let Some(safety) = safety {
        structured["safety"] = safety;
    }
    json!({
        "content": [{ "type": "text", "text": format!("Error: {message}") }],
        "structuredContent": structured,
        "isError": true,
    })
}

fn refused(e: ApiError, invocation_id: Option<Uuid>) -> Value {
    let code = e.code();
    refusal(code, &e.to_string(), invocation_id, None)
}

/// A capability's input schema as a tool's: an object schema as it is (given
/// `type: object` when it says nothing else); anything else wrapped as
/// `{input}`, and unwrapped on call.
fn input_schema(schema: &Value) -> (Value, bool) {
    let Some(map) = schema.as_object() else {
        return (wrap(schema), true);
    };
    match map.get("type") {
        Some(t) if t == "object" => (schema.clone(), false),
        None if map.is_empty() || map.contains_key("properties") => {
            let mut s = schema.clone();
            s["type"] = json!("object");
            (s, false)
        }
        _ => (wrap(schema), true),
    }
}

fn wrap(schema: &Value) -> Value {
    json!({ "type": "object", "properties": { "input": schema }, "required": ["input"] })
}

/// Every tool's output schema: the call's status, and the capability's own
/// output schema for `output`. Each result, finished, pending or refused,
/// conforms.
fn output_schema(capability_output: &Value) -> Value {
    json!({
        "type": "object",
        "required": ["status"],
        "properties": {
            "status": {
                "type": "string",
                "enum": ["succeeded", "pending", "in_progress", "awaiting_consent", "failed", "timeout", "refused"]
            },
            "invocation_id": { "type": "string" },
            "output": capability_output,
            "code": { "type": "string" },
            "error": { "type": "string" },
            "safety": { "type": "object" },
            "peer": { "type": "object" }
        }
    })
}

/// A tool name for a capability: `.` and anything else MCP doesn't allow
/// become `_`, cut to 64 characters, with `_2`, `_3`… on a collision.
fn unique_tool_name(capability: &str, taken: &mut HashSet<String>) -> String {
    let base: String = capability
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(MAX_TOOL_NAME)
        .collect();
    let base = if base.is_empty() {
        "capability".to_owned()
    } else {
        base
    };
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&name) {
        let suffix = format!("_{n}");
        let keep: String = base.chars().take(MAX_TOOL_NAME - suffix.len()).collect();
        name = format!("{keep}{suffix}");
        n += 1;
    }
    taken.insert(name.clone());
    name
}

/// The sign-in metadata for an agent's endpoint (RFC 9728), served at
/// `/.well-known/oauth-protected-resource/agents/{a}/{b}/mcp`.
pub async fn protected_resource_metadata(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
) -> axum::Json<Value> {
    axum::Json(json!({
        "resource": format!(
            "{}/agents/{account_slug}/{agent_slug}/mcp",
            state.config.relay_base_url.trim_end_matches('/')
        ),
        "authorization_servers": [state.config.app_base_url],
        "scopes_supported": ["relay.full"],
        "bearer_methods_supported": ["header"],
    }))
}

/// The metadata URL a 401 on `path` names: an agent endpoint's own, else
/// `/mcp`'s.
pub fn metadata_url(relay_base_url: &str, path: &str) -> String {
    let base = relay_base_url.trim_end_matches('/');
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        ["agents", account, agent, "mcp"] => {
            format!("{base}/.well-known/oauth-protected-resource/agents/{account}/{agent}/mcp")
        }
        _ => format!("{base}/.well-known/oauth-protected-resource"),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn tool_names_are_mcp_shaped_and_unique() {
        let mut taken: HashSet<String> = [ABOUT.to_owned()].into();
        assert_eq!(unique_tool_name("guide.ask", &mut taken), "guide_ask");
        assert_eq!(unique_tool_name("guide_ask", &mut taken), "guide_ask_2");
        assert_eq!(unique_tool_name("guide ask", &mut taken), "guide_ask_3");
        assert_eq!(
            unique_tool_name("chakramcp_about", &mut taken),
            "chakramcp_about_2"
        );
        let long = "x".repeat(80);
        let first = unique_tool_name(&long, &mut taken);
        let second = unique_tool_name(&long, &mut taken);
        assert_eq!(first.len(), 64);
        assert_eq!(second.len(), 64);
        assert!(second.ends_with("_2"));
    }

    #[test]
    fn non_object_inputs_are_wrapped() {
        assert!(!input_schema(&json!({ "type": "object" })).1);
        let (filled, wrapped) = input_schema(&json!({}));
        assert_eq!((filled["type"].clone(), wrapped), (json!("object"), false));
        let (s, wrapped) = input_schema(&json!({ "type": "string" }));
        assert!(wrapped);
        assert_eq!(s["required"], json!(["input"]));
        assert!(input_schema(&json!({ "type": ["object", "null"] })).1);
    }

    #[test]
    fn challenges_name_the_endpoint() {
        assert_eq!(
            metadata_url("https://relay.example/", "/agents/bob/venue-bot/mcp"),
            "https://relay.example/.well-known/oauth-protected-resource/agents/bob/venue-bot/mcp"
        );
        assert_eq!(
            metadata_url("https://relay.example", "/mcp"),
            "https://relay.example/.well-known/oauth-protected-resource"
        );
    }
}
