//! `chakramcp discover` — search the public agent directory.
//!
//! Wraps GET /v1/discovery/agents (D10a) so an operator (or an agent
//! shelling out from Claude Code) can find peers without going to the
//! web UI. The response shape matches the relay verbatim:
//!
//!     { agents: [...], next_cursor: null | string, total_estimate?: int }
//!
//! Authentication is optional — the discovery endpoint is public —
//! but if a token is configured, it's sent through. That's harmless
//! and gives the relay an audit trail of who searched for what.
//!
//! Filter mix matches the HTTP API; see the docs page for the full
//! semantics. Common shapes:
//!
//!     chakramcp discover                            # all network agents
//!     chakramcp discover -q "git history"           # full-text query
//!     chakramcp discover --capability check_worklog # exact name match
//!     chakramcp discover --tags scheduling,calendar # tag AND-filter
//!     chakramcp discover --mode pull --verified     # type + trust
//!     chakramcp discover --cursor <opaque>          # next page

use anyhow::Result;
use clap::Args;
use serde_json::Value;

use crate::client::ApiClient;
use crate::print;

#[derive(Args, Debug)]
pub struct DiscoverArgs {
    /// Full-text query against display name, description, and tags.
    #[arg(long, short = 'q')]
    pub query: Option<String>,

    /// Filter to agents that publish a capability with this name.
    /// Exact match against `agent_capabilities.name`.
    #[arg(long)]
    pub capability: Option<String>,

    /// `push` or `pull`. Omit for both.
    #[arg(long)]
    pub mode: Option<String>,

    /// Only show agents whose owning account is verified.
    #[arg(long)]
    pub verified: bool,

    /// Comma-separated tags. Matched as AND (every tag must be present).
    #[arg(long)]
    pub tags: Option<String>,

    /// Include agents that haven't been seen by the relay recently.
    /// By default the directory hides them to keep results trustworthy.
    #[arg(long)]
    pub include_dormant: bool,

    /// Page size. Default 20, max 100.
    #[arg(long, default_value_t = 20)]
    pub limit: u32,

    /// Opaque cursor from a previous response's `next_cursor` field.
    #[arg(long)]
    pub cursor: Option<String>,
}

pub async fn run(args: DiscoverArgs, api: ApiClient) -> Result<()> {
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(q) = args.query {
        params.push(("q".into(), q));
    }
    if let Some(c) = args.capability {
        params.push(("capability".into(), c));
    }
    if let Some(m) = args.mode {
        params.push(("mode".into(), m));
    }
    if args.verified {
        params.push(("verified".into(), "true".into()));
    }
    if let Some(t) = args.tags {
        params.push(("tags".into(), t));
    }
    if args.include_dormant {
        params.push(("include_dormant".into(), "true".into()));
    }
    params.push(("limit".into(), args.limit.to_string()));
    if let Some(c) = args.cursor {
        params.push(("cursor".into(), c));
    }

    let qs: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish();

    let path = if qs.is_empty() {
        "/v1/discovery/agents".to_string()
    } else {
        format!("/v1/discovery/agents?{qs}")
    };
    let body: Value = api.get_relay_optional_auth(&path).await?;
    print(&body)
}
