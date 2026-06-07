//! `chakramcp audit` — your write/audit trail.
//!
//! Wraps GET /v1/audit (migration 0025). Shows create/update/delete/
//! state-change events across your accounts (agents, capabilities,
//! friendships, grants, reviews) — newest first. Read-only actions are
//! intentionally not in the audit log; see `chakramcp usage` for those.
//!
//!     chakramcp audit                          # latest events
//!     chakramcp audit --resource-type grant    # only grant events
//!     chakramcp audit --action friendship.accept
//!     chakramcp audit --cursor <opaque>        # next page

use anyhow::Result;
use clap::Args;
use serde_json::Value;

use crate::client::ApiClient;
use crate::print;

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Page size. Default 50, max 200.
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Filter to one resource type: agent | capability | friendship | grant | review.
    #[arg(long)]
    pub resource_type: Option<String>,

    /// Filter to one action, e.g. `grant.revoke`.
    #[arg(long)]
    pub action: Option<String>,

    /// Opaque cursor from a previous response's `next_cursor`.
    #[arg(long)]
    pub cursor: Option<String>,
}

pub async fn run(args: AuditArgs, api: ApiClient) -> Result<()> {
    let mut params: Vec<(String, String)> = vec![("limit".into(), args.limit.to_string())];
    if let Some(r) = args.resource_type {
        params.push(("resource_type".into(), r));
    }
    if let Some(a) = args.action {
        params.push(("action".into(), a));
    }
    if let Some(c) = args.cursor {
        params.push(("cursor".into(), c));
    }
    let qs: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish();
    let body: Value = api.get_relay(&format!("/v1/audit?{qs}")).await?;
    print(&body)
}
