//! `chakramcp usage` — your metered request history.
//!
//! Wraps GET /v1/usage/events (migration 0025). Every API request you
//! make — REST and per MCP tool, reads included — is metered here, the
//! substrate for future billing. The response carries `totals_by_action`
//! and a grand `total` as a billing preview.
//!
//!     chakramcp usage                  # latest events + totals
//!     chakramcp usage --surface mcp    # only MCP tool calls
//!     chakramcp usage --cursor <opaque>

use anyhow::Result;
use clap::Args;
use serde_json::Value;

use crate::client::ApiClient;
use crate::print;

#[derive(Args, Debug)]
pub struct UsageArgs {
    /// Page size. Default 50, max 200.
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Filter by surface: `rest` or `mcp`.
    #[arg(long)]
    pub surface: Option<String>,

    /// Opaque cursor from a previous response's `next_cursor`.
    #[arg(long)]
    pub cursor: Option<String>,
}

pub async fn run(args: UsageArgs, api: ApiClient) -> Result<()> {
    let mut params: Vec<(String, String)> = vec![("limit".into(), args.limit.to_string())];
    if let Some(s) = args.surface {
        params.push(("surface".into(), s));
    }
    if let Some(c) = args.cursor {
        params.push(("cursor".into(), c));
    }
    let qs: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish();
    let body: Value = api.get_relay(&format!("/v1/usage/events?{qs}")).await?;
    print(&body)
}
