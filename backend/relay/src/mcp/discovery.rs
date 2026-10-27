//! The discovery tools (MCP and discovery spec §6.2–6.3), shared by `/mcp`
//! (signed in: your private agents and your standing, `you`) and
//! `/mcp/directory` (signed out: the public directory).

use serde::Deserialize;
use serde_json::{json, Value};

use chakramcp_shared::error::ApiError;

use super::rpc::tool;
use crate::auth::AuthUser;
use crate::discovery::profile::{self, AgentRef};
use crate::discovery::search::{self, SearchParams};
use crate::state::RelayState;

/// How many reviews `list_reviews` returns.
const REVIEWS_LIMIT: i64 = 50;

/// `search_agents`; `callable` is offered only to a signed-in caller.
pub fn search_tool(signed_in: bool) -> Value {
    let mut properties = json!({
        "q": { "type": "string", "description": "Free text, e.g. 'book a meeting room'." },
        "capability": { "type": "string", "description": "Part of a capability's name." },
        "tags": { "type": "array", "items": { "type": "string" }, "description": "Agents with all of these tags." },
        "mode": { "type": "string", "enum": ["push", "pull"] },
        "include_dormant": { "type": "boolean", "description": "Also agents not seen in 30 days with nothing public." },
        "limit": { "type": "integer", "minimum": 1, "maximum": 100 },
        "cursor": { "type": "string" }
    });
    let description = if signed_in {
        properties["callable"] =
            json!({ "type": "boolean", "description": "Only agents you can call now." });
        "Search the agent directory, best matches first: any word of q matches, and a name or slug match ranks highest. Results include your own private agents, what each offers (capabilities, with access 'public' or 'friends'), whether it's online, and your standing (you.callable lists what you can call now). Pass next_cursor back as cursor, with the same filters, for more."
    } else {
        "Search the public agent directory, best matches first: any word of q matches, and a name or slug match ranks highest. Each result says what the agent offers (capabilities, with access 'public' for anyone signed in, or 'friends') and whether it's online. Pass next_cursor back as cursor, with the same filters, for more."
    };
    tool(
        "search_agents",
        description,
        json!({ "type": "object", "properties": properties }),
    )
}

pub fn featured_tool() -> Value {
    tool(
        "featured_agents",
        "Agents worth trying first: the relay's own house agents, then online agents with something anyone can call.",
        json!({ "type": "object", "properties": {} }),
    )
}

pub fn profile_tool(signed_in: bool) -> Value {
    tool(
        "get_agent_profile",
        if signed_in {
            "Read one agent's profile: what it does, its owner, its capabilities with their input and output schemas and who can call each, reviews, how to call it, and your standing with it (what you can call now, and as which of your agents)."
        } else {
            "Read one agent's profile: what it does, its owner, its capabilities with their input and output schemas and who can call each, its latest reviews, and how to call it."
        },
        json!({
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string", "description": "The agent's id, or account/agent (e.g. bob/venue-bot)." }
            }
        }),
    )
}

pub fn reviews_tool() -> Value {
    tool(
        "list_reviews",
        "An agent's reviews, newest first: rating, text, when, and by which agent.",
        json!({
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string", "description": "The agent's id, or account/agent." }
            }
        }),
    )
}

/// An agent named by id or `account/agent` in `args[key]`.
pub fn agent_ref_arg(args: &Value, key: &str) -> Result<AgentRef, ApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .and_then(AgentRef::parse)
        .ok_or_else(|| {
            ApiError::InvalidRequest(format!("{key} must be an agent id or account/agent"))
        })
}

/// `get_agent_profile`.
pub async fn get_agent_profile(
    state: &RelayState,
    viewer: Option<&AuthUser>,
    args: &Value,
) -> Result<Value, ApiError> {
    let reference = agent_ref_arg(args, "agent")?;
    let loaded = profile::resolve(state, viewer.map(|u| u.user_id), &reference)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(json!(profile::profile(state, viewer, loaded).await?))
}

/// `search_agents`.
pub async fn search_agents(
    state: &RelayState,
    viewer: Option<&AuthUser>,
    args: Value,
) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    struct A {
        q: Option<String>,
        capability: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        mode: Option<String>,
        callable: Option<bool>,
        include_dormant: Option<bool>,
        limit: Option<i64>,
        cursor: Option<String>,
    }
    let a: A = serde_json::from_value(args)
        .map_err(|e| ApiError::InvalidRequest(format!("bad search_agents args: {e}")))?;
    let page = search::search(
        state,
        viewer,
        SearchParams {
            q: a.q,
            capability: a.capability,
            tags: (!a.tags.is_empty()).then(|| a.tags.join(",")),
            mode: a.mode,
            callable: a.callable,
            include_dormant: a.include_dormant,
            limit: a.limit,
            cursor: a.cursor,
            ..Default::default()
        },
    )
    .await?;
    Ok(json!(page))
}

/// `featured_agents`.
pub async fn featured_agents(
    state: &RelayState,
    viewer: Option<&AuthUser>,
) -> Result<Value, ApiError> {
    let agents = crate::discovery::featured::featured(state, viewer.map(|u| u.user_id)).await?;
    Ok(json!({ "agents": agents }))
}

/// `list_reviews {agent}`: the reviews of an agent the viewer may see.
pub async fn list_reviews(
    state: &RelayState,
    viewer: Option<&AuthUser>,
    args: &Value,
) -> Result<Value, ApiError> {
    let reference = agent_ref_arg(args, "agent")?;
    let loaded = profile::resolve(state, viewer.map(|u| u.user_id), &reference)
        .await?
        .ok_or(ApiError::NotFound)?;
    let reviews = profile::reviews(state, loaded.summary.id, REVIEWS_LIMIT).await?;
    Ok(json!({
        "agent": format!("{}/{}", loaded.summary.account_slug, loaded.summary.slug),
        "avg_rating": loaded.summary.avg_rating,
        "count": loaded.summary.review_count,
        "reviews": reviews,
    }))
}
