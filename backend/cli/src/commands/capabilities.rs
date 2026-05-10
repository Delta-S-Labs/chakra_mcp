//! `chakramcp capabilities` — manage what an agent can do.
//!
//! Wraps `/v1/agents/{id}/capabilities` (list + create) and
//! `/v1/agents/{id}/capabilities/{cap_id}` (update + delete). Schemas
//! can be passed inline as JSON strings or read from files via the
//! @path syntax (matches `chakramcp invoke` for consistency).
//!
//!     chakramcp capabilities list --agent <id>
//!     chakramcp capabilities add --agent <id> \
//!         --name check_worklog \
//!         --description "Summarize git activity in a date range" \
//!         --input-schema  @schemas/check_worklog.input.json \
//!         --output-schema @schemas/check_worklog.output.json
//!     chakramcp capabilities update --agent <id> --cap <cap_id> \
//!         --description "..."
//!     chakramcp capabilities delete --agent <id> --cap <cap_id>
//!
//! Visibility defaults to `network` when adding (so peers can
//! discover via `chakramcp discover`). Pass `--visibility private`
//! to hide it — useful for in-progress capabilities you only want
//! direct grant-holders to see.

use std::fs;

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List the capabilities published by one of your agents.
    List {
        #[arg(long)]
        agent: String,
    },
    /// Publish a new capability on one of your agents.
    Add {
        #[arg(long)]
        agent: String,
        /// Snake_case name. Becomes the `capability_name` in
        /// invocations and the slug peers see.
        #[arg(long)]
        name: String,
        /// One-line description. Shows up in /agents detail.
        #[arg(long, default_value = "")]
        description: String,
        /// JSON Schema for the input payload. Pass inline JSON or
        /// `@path/to/file.json` to read from disk.
        #[arg(long)]
        input_schema: String,
        /// JSON Schema for the output. Same `@file` syntax allowed.
        #[arg(long)]
        output_schema: String,
        /// `network` (default) or `private`.
        #[arg(long, default_value = "network")]
        visibility: String,
    },
    /// Patch a capability's metadata (description, visibility, schemas).
    Update {
        #[arg(long)]
        agent: String,
        #[arg(long, alias = "capability")]
        cap: String,
        /// New description, if changing.
        #[arg(long)]
        description: Option<String>,
        /// New visibility (`network` or `private`).
        #[arg(long)]
        visibility: Option<String>,
        /// Replace the input schema (inline JSON or `@file`).
        #[arg(long)]
        input_schema: Option<String>,
        /// Replace the output schema (inline JSON or `@file`).
        #[arg(long)]
        output_schema: Option<String>,
    },
    /// Soft-delete a capability. Existing grants are revoked.
    Delete {
        #[arg(long)]
        agent: String,
        #[arg(long, alias = "capability")]
        cap: String,
    },
}

/// Parse `@path/to/file.json` or inline JSON into a Value.
/// Same convention `chakramcp invoke --input @file` uses.
fn parse_schema(arg: &str, label: &str) -> Result<Value> {
    let raw = if let Some(path) = arg.strip_prefix('@') {
        fs::read_to_string(path).with_context(|| format!("reading {label} from {path}"))?
    } else {
        arg.to_string()
    };
    serde_json::from_str(&raw).with_context(|| format!("parsing {label} as JSON"))
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List { agent } => {
            let body: Value = api
                .get_relay(&format!("/v1/agents/{agent}/capabilities"))
                .await?;
            print(&body)
        }
        Cmd::Add {
            agent,
            name,
            description,
            input_schema,
            output_schema,
            visibility,
        } => {
            let input = parse_schema(&input_schema, "input_schema")?;
            let output = parse_schema(&output_schema, "output_schema")?;
            let payload = json!({
                "name": name,
                "description": description,
                "input_schema": input,
                "output_schema": output,
                "visibility": visibility,
            });
            let body: Value = api
                .post_relay(&format!("/v1/agents/{agent}/capabilities"), &payload)
                .await?;
            print(&body)
        }
        Cmd::Update {
            agent,
            cap,
            description,
            visibility,
            input_schema,
            output_schema,
        } => {
            let mut payload = serde_json::Map::new();
            if let Some(d) = description {
                payload.insert("description".into(), Value::String(d));
            }
            if let Some(v) = visibility {
                payload.insert("visibility".into(), Value::String(v));
            }
            if let Some(s) = input_schema {
                payload.insert("input_schema".into(), parse_schema(&s, "input_schema")?);
            }
            if let Some(s) = output_schema {
                payload.insert("output_schema".into(), parse_schema(&s, "output_schema")?);
            }
            if payload.is_empty() {
                return Err(anyhow!(
                    "nothing to update — pass at least one of --description, --visibility, --input-schema, --output-schema"
                ));
            }
            let body: Value = api
                .patch_relay(
                    &format!("/v1/agents/{agent}/capabilities/{cap}"),
                    &Value::Object(payload),
                )
                .await?;
            print(&body)
        }
        Cmd::Delete { agent, cap } => {
            let _: Value = api
                .delete_relay(&format!("/v1/agents/{agent}/capabilities/{cap}"))
                .await?;
            print(&json!({ "deleted": true, "agent_id": agent, "capability_id": cap }))
        }
    }
}
