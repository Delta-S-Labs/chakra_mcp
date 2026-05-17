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
use crate::templates::{get_template, known_templates};

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List the capabilities published by one of your agents.
    List {
        #[arg(long)]
        agent: String,
    },
    /// Publish a new capability on one of your agents.
    ///
    /// Two ways to invoke:
    ///   * `--template <id>` — use a reserved-name capability with
    ///     its canonical schema (e.g. `--template message_owner`).
    ///     Mutually exclusive with the manual flags below.
    ///   * `--name + --input-schema + --output-schema` — custom
    ///     capability. The full triple is required when not using
    ///     a template.
    Add {
        #[arg(long)]
        agent: String,
        /// Reserved-template id. When set, `--name` /
        /// `--input-schema` / `--output-schema` are ignored
        /// (template wins) — but `--description` and
        /// `--visibility` still override the template defaults.
        /// Run `chakramcp capabilities templates` to list.
        #[arg(long, conflicts_with_all = ["name", "input_schema", "output_schema"])]
        template: Option<String>,
        /// Snake_case name. Required unless `--template` is set.
        #[arg(long, required_unless_present = "template")]
        name: Option<String>,
        /// One-line description. Optional override even with a template.
        #[arg(long)]
        description: Option<String>,
        /// JSON Schema for the input. Required unless `--template`.
        /// Pass inline JSON or `@path/to/file.json`.
        #[arg(long, required_unless_present = "template")]
        input_schema: Option<String>,
        /// JSON Schema for the output. Required unless `--template`.
        #[arg(long, required_unless_present = "template")]
        output_schema: Option<String>,
        /// `network` (default) or `private`. Overrides template default.
        #[arg(long)]
        visibility: Option<String>,
        /// HITL gate (issue #69 PR 2). `autonomous` (default) accepts
        /// any worker-posted result; `human_in_loop` makes the relay
        /// reject results that don't carry `confirmed_by_human: true`.
        /// The `message_owner` template defaults to `human_in_loop`;
        /// passing this flag overrides the template's value.
        #[arg(long, value_parser = ["autonomous", "human_in_loop"])]
        semantics: Option<String>,
    },
    /// List the reserved capability templates available with
    /// `add --template <id>`. Names are stable across SDK + CLI;
    /// schemas live in /docs/agents.
    Templates,
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
            template,
            name,
            description,
            input_schema,
            output_schema,
            visibility,
            semantics,
        } => {
            let mut payload = if let Some(tpl_id) = template {
                // Template path: load the canonical body, then apply
                // optional description / visibility / semantics overrides.
                get_template(&tpl_id).ok_or_else(|| {
                    anyhow!(
                        "unknown template id: '{tpl_id}'. Known: {}. Run `chakramcp capabilities templates` to list.",
                        known_templates().join(", ")
                    )
                })?
            } else {
                // Manual path: clap's `required_unless_present` guarantees
                // these are Some when we get here.
                let name = name.expect("clap required_unless_present");
                let input_schema = input_schema.expect("clap required_unless_present");
                let output_schema = output_schema.expect("clap required_unless_present");
                let input = parse_schema(&input_schema, "input_schema")?;
                let output = parse_schema(&output_schema, "output_schema")?;
                json!({
                    "name": name,
                    "description": description.clone().unwrap_or_default(),
                    "input_schema": input,
                    "output_schema": output,
                    "visibility": visibility.clone().unwrap_or_else(|| "network".to_string()),
                    // Default matches the migration. The relay also
                    // defaults at the DB layer, but sending it
                    // explicitly keeps the wire shape obvious to
                    // anyone tracing requests.
                    "semantics": semantics.clone().unwrap_or_else(|| "autonomous".to_string()),
                })
            };
            // Overrides apply in both paths.
            if let Some(d) = description {
                payload["description"] = Value::String(d);
            }
            if let Some(v) = visibility {
                payload["visibility"] = Value::String(v);
            }
            if let Some(s) = semantics {
                payload["semantics"] = Value::String(s);
            }
            let body: Value = api
                .post_relay(&format!("/v1/agents/{agent}/capabilities"), &payload)
                .await?;
            print(&body)
        }
        Cmd::Templates => {
            let templates: Vec<Value> = known_templates()
                .iter()
                .filter_map(|id| {
                    get_template(id).map(|body| {
                        json!({
                            "id": id,
                            "name": body.get("name"),
                            "description": body.get("description"),
                        })
                    })
                })
                .collect();
            print(&json!({ "templates": templates }))
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
