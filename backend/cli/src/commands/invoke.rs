use std::time::Duration;

use anyhow::{bail, Result};
use clap::Parser;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::{print, read_json_arg};

const POLL_INTERVAL_MS: u64 = 1500;
const POLL_MAX_ATTEMPTS: usize = 120; // 3 minutes

#[derive(Parser, Debug)]
pub struct Args {
    /// Grant id authorising this call.
    #[arg(long)]
    pub grant: String,
    /// Your agent that's making the call (the grantee side).
    #[arg(long = "as")]
    pub as_agent: String,
    /// Input — JSON literal, or `@path` to read from a file, or `@-` for stdin.
    #[arg(long, default_value = "{}")]
    pub input: String,
    /// Poll until the invocation reaches a terminal status, then print the row.
    #[arg(long)]
    pub wait: bool,
}

pub async fn run(args: Args, api: ApiClient) -> Result<()> {
    let input = read_json_arg(&args.input)?;
    let body = json!({
        "grant_id": args.grant,
        "grantee_agent_id": args.as_agent,
        "input": input,
    });
    let resp: Value = api.post_relay("/v1/invoke", &body).await?;

    if !args.wait {
        return print(&resp);
    }

    let id = resp
        .get("invocation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("invoke response missing invocation_id"))?
        .to_string();

    eprintln!("Enqueued {id}. Waiting for terminal status…");
    for attempt in 0..POLL_MAX_ATTEMPTS {
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        let row: Value = api.get_relay(&format!("/v1/invocations/{id}")).await?;
        let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(status, "succeeded" | "failed" | "rejected" | "timeout") {
            return print(&row);
        }
        if attempt > 0 && attempt % 10 == 0 {
            eprintln!("  still {status} after {}s…", (attempt + 1) * (POLL_INTERVAL_MS as usize) / 1000);
        }
    }
    bail!("timed out after 3 minutes — re-run `chakramcp invoke wait` or check `chakramcp inbox` on the granter side");
}
