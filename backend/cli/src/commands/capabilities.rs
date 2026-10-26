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

/// Owner consent settings, on `add` and `update`.
#[derive(clap::Args, Debug, Default, Clone)]
pub struct ConsentFlags {
    /// Hold every request until one of the agent's owners approves it on
    /// the web; the agent gets a notice with the link (`inbox notices`).
    /// Pull-mode agents only.
    #[arg(long, conflicts_with = "no_ask_first")]
    pub ask_first: bool,
    /// Stop asking first. Requests already held still wait for a decision.
    #[arg(long)]
    pub no_ask_first: bool,
    /// Who may approve: a member's email or user id (repeat for more).
    /// Without it, the account's owners and admins decide.
    #[arg(long = "approver")]
    pub approvers: Vec<String>,
    /// The longest standing consent an approver may grant.
    #[arg(long, value_parser = ["once", "1h", "1d", "7d", "30d"])]
    pub max_consent: Option<String>,
    /// How long a held request waits for a decision: `<n>m`, `<n>h` or
    /// `<n>d`, from 5m to 7d (default 24h).
    #[arg(long)]
    pub consent_wait: Option<String>,
}

impl ConsentFlags {
    /// The `consent` object for the request body, if any flag was given.
    pub fn to_json(&self) -> Option<Value> {
        let mut consent = serde_json::Map::new();
        if self.ask_first {
            consent.insert("required".into(), Value::Bool(true));
        } else if self.no_ask_first {
            consent.insert("required".into(), Value::Bool(false));
        }
        if !self.approvers.is_empty() {
            consent.insert("approvers".into(), json!(self.approvers));
        }
        if let Some(v) = &self.max_consent {
            consent.insert("max_validity".into(), json!(v));
        }
        if let Some(w) = &self.consent_wait {
            consent.insert("wait".into(), json!(w));
        }
        (!consent.is_empty()).then_some(Value::Object(consent))
    }
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List an agent's capabilities, with their schemas: all of your own
    /// agent's, or the network-visible ones of anyone else's.
    List {
        /// The agent: its id, `account/agent`, or a slug.
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
        /// `autonomous` (default): the agent's replies go out as they are.
        /// `human_in_loop`: a reply is a draft until one of the agent's
        /// owners confirms it on the web (or writes it there). The
        /// `message_owner` template defaults to `human_in_loop`; passing
        /// this flag overrides the template's value.
        #[arg(long, value_parser = ["autonomous", "human_in_loop"])]
        semantics: Option<String>,
        /// Make this capability publicly invokable: any registered
        /// agent on the network can call it without a friendship/grant
        /// (migration 0022). Requires `--visibility network` (default)
        /// and a non-zero `--monthly-quota`. The relay enforces a
        /// per-invoker monthly cap; exceeders get a 429.
        #[arg(long)]
        public_invoke: bool,
        /// Per-invoker monthly cap on this capability (required when
        /// `--public-invoke` is set; ignored otherwise). Calendar
        /// month, resets on the 1st.
        #[arg(long)]
        monthly_quota: Option<i32>,
        #[command(flatten)]
        consent: ConsentFlags,
    },
    /// List the reserved capability templates available with
    /// `add --template <id>`. Names are stable across SDK + CLI;
    /// schemas live in /docs/agents.
    Templates,
    /// Patch a capability's metadata (description, visibility, schemas,
    /// public-invoke + quota).
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
        /// Turn public-invoke ON for this capability (migration 0022).
        /// Mutually exclusive with `--no-public-invoke`. Requires the
        /// effective visibility to be `network` and a non-zero quota
        /// (set on this same command or already on the row).
        #[arg(long, conflicts_with = "no_public_invoke")]
        public_invoke: bool,
        /// Turn public-invoke OFF for this capability.
        #[arg(long)]
        no_public_invoke: bool,
        /// Set the per-invoker monthly quota. Required when enabling
        /// public-invoke; otherwise updates the cap.
        #[arg(long)]
        monthly_quota: Option<i32>,
        /// `human_in_loop`: a person confirms each reply on the web before
        /// the caller sees it. `autonomous`: the agent's replies go out as
        /// they are.
        #[arg(long, value_parser = ["autonomous", "human_in_loop"])]
        semantics: Option<String>,
        #[command(flatten)]
        consent: ConsentFlags,
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
            let agent = crate::resolve::peer_id(&api, &agent).await?;
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
            public_invoke,
            monthly_quota,
            consent,
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
            // Public-invoke + quota. The relay enforces the
            // (network-only) + (quota required when on) invariants;
            // we just plumb the values through.
            if public_invoke {
                if monthly_quota.is_none() {
                    return Err(anyhow!("--public-invoke requires --monthly-quota <N>"));
                }
                payload["public_invoke"] = Value::Bool(true);
            }
            if let Some(q) = monthly_quota {
                payload["public_monthly_quota_per_agent"] = json!(q);
            }
            if let Some(c) = consent.to_json() {
                payload["consent"] = c;
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
            public_invoke,
            no_public_invoke,
            monthly_quota,
            semantics,
            consent,
        } => {
            let mut payload = serde_json::Map::new();
            if let Some(s) = semantics {
                payload.insert("semantics".into(), Value::String(s));
            }
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
            // Public-invoke toggle: `--public-invoke` xor
            // `--no-public-invoke` (clap enforces the conflict).
            if public_invoke {
                payload.insert("public_invoke".into(), Value::Bool(true));
            } else if no_public_invoke {
                payload.insert("public_invoke".into(), Value::Bool(false));
            }
            if let Some(q) = monthly_quota {
                payload.insert("public_monthly_quota_per_agent".into(), json!(q));
            }
            if let Some(c) = consent.to_json() {
                payload.insert("consent".into(), c);
            }
            if payload.is_empty() {
                return Err(anyhow!(
                    "nothing to update — pass at least one of --description, --visibility, --input-schema, --output-schema, --public-invoke, --no-public-invoke, --monthly-quota, --semantics, --ask-first, --no-ask-first, --approver, --max-consent, --consent-wait"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_flags_build_the_consent_object() {
        assert_eq!(ConsentFlags::default().to_json(), None);
        let flags = ConsentFlags {
            ask_first: true,
            approvers: vec!["ada@example.test".into()],
            max_consent: Some("7d".into()),
            consent_wait: Some("2h".into()),
            ..ConsentFlags::default()
        };
        assert_eq!(
            flags.to_json(),
            Some(json!({
                "required": true,
                "approvers": ["ada@example.test"],
                "max_validity": "7d",
                "wait": "2h",
            }))
        );
        let off = ConsentFlags {
            no_ask_first: true,
            ..ConsentFlags::default()
        };
        assert_eq!(off.to_json(), Some(json!({"required": false})));
    }
}
