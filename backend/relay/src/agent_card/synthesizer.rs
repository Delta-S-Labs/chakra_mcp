//! Pull-mode card synthesis.
//!
//! For a pull-mode agent (no public A2A endpoint, runs `inbox.serve()`
//! against our inbox bridge), we have no upstream Agent Card to fetch.
//! Instead we synthesize a card from registration data:
//!
//! - `name` / `description` from the agents row
//! - `url` from the relay base URL + the agent's slug pair (always our
//!   relay endpoint)
//! - `skills` from agent_capabilities rows (one skill per capability)
//! - `authentication` always `[ http+bearer ]` per discovery spec §"Override #4"
//! - `capabilities.streaming = false` for v1; the inbox bridge can
//!   simulate streaming behavior later via Task heartbeats.
//!
//! Push-mode synthesis is in `fetcher.rs` (D2d). The signing happens in
//! `signer.rs` (D2b) — this module returns an *unsigned* card.

use super::types::{
    AgentAuthentication, AgentCapabilities, AgentCard, AgentSkill, JsonSchema,
};

/// Minimal projection of an `agents` row needed for synthesis.
///
/// Constructing this from a sqlx-fetched row keeps the synthesizer
/// free of DB types and easy to unit-test.
#[derive(Debug, Clone)]
pub struct AgentRowForSynthesis {
    pub account_slug: String,
    pub agent_slug: String,
    pub display_name: String,
    pub description: String,
}

/// Minimal projection of an `agent_capabilities` row.
#[derive(Debug, Clone)]
pub struct CapabilityRowForSynthesis {
    /// We use the capability UUID as the A2A skill `id` so policy
    /// lookups at call time don't need a name → id round-trip.
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
}

/// Synthesize an unsigned Agent Card for a pull-mode agent.
///
/// `relay_base_url` is the public-facing host where this card will be
/// served (e.g. `"https://chakramcp.com"`). The synthesized `url` is
/// `<base>/agents/<account>/<slug>/a2a/jsonrpc`.
///
/// The caller (D2c HTTP handler) is expected to sign the result via
/// `signer::sign_card` before serving.
pub fn synthesize_pull_card(
    agent: &AgentRowForSynthesis,
    capabilities: &[CapabilityRowForSynthesis],
    relay_base_url: &str,
) -> AgentCard {
    // Trim trailing slash off the base so URL composition is unambiguous.
    let base = relay_base_url.trim_end_matches('/');
    let url = format!(
        "{base}/agents/{}/{}/a2a/jsonrpc",
        agent.account_slug, agent.agent_slug
    );

    let skills = capabilities
        .iter()
        .map(|c| AgentSkill {
            id: c.id.clone(),
            name: c.name.clone(),
            description: if c.description.is_empty() {
                None
            } else {
                Some(c.description.clone())
            },
            input_schema: c.input_schema.clone(),
            output_schema: c.output_schema.clone(),
        })
        .collect();

    AgentCard {
        name: agent.display_name.clone(),
        description: if agent.description.is_empty() {
            None
        } else {
            Some(agent.description.clone())
        },
        url,
        version: "0.3".to_string(),
        skills,
        authentication: vec![bearer_jwt_auth_scheme()],
        capabilities: Some(AgentCapabilities {
            streaming: false,
            push_notifications: false,
        }),
        signature: None,
        original_signature: None,
    }
}

/// The single auth scheme ChakraMCP publishes on every card — see
/// discovery spec §"Override #4".
fn bearer_jwt_auth_scheme() -> AgentAuthentication {
    AgentAuthentication {
        scheme: "http".to_string(),
        auth_type: "bearer".to_string(),
        bearer_format: Some("JWT".to_string()),
        description: Some(
            "ChakraMCP-issued bearer token (API key or OAuth-issued JWT).".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(slug: &str) -> AgentRowForSynthesis {
        AgentRowForSynthesis {
            account_slug: "acme-corp".into(),
            agent_slug: slug.into(),
            display_name: "Alice Scheduler".into(),
            description: "Returns 30-min slots in the next N days.".into(),
        }
    }

    fn capability(id: &str, name: &str) -> CapabilityRowForSynthesis {
        CapabilityRowForSynthesis {
            id: id.into(),
            name: name.into(),
            description: "Plan a thing.".into(),
            input_schema: json!({"type":"object","required":["duration_min"]}),
            output_schema: json!({"type":"object","required":["slots"]}),
        }
    }

    #[test]
    fn url_uses_path_based_relay_endpoint() {
        let card = synthesize_pull_card(
            &agent("alice"),
            &[],
            "https://chakramcp.com",
        );
        assert_eq!(
            card.url,
            "https://chakramcp.com/agents/acme-corp/alice/a2a/jsonrpc"
        );
    }

    #[test]
    fn trailing_slash_on_base_is_normalized() {
        let card = synthesize_pull_card(&agent("alice"), &[], "https://chakramcp.com/");
        assert_eq!(
            card.url,
            "https://chakramcp.com/agents/acme-corp/alice/a2a/jsonrpc"
        );
    }

    #[test]
    fn always_bearer_jwt_only() {
        let card = synthesize_pull_card(&agent("alice"), &[], "https://r");
        assert_eq!(card.authentication.len(), 1);
        let a = &card.authentication[0];
        assert_eq!(a.scheme, "http");
        assert_eq!(a.auth_type, "bearer");
        assert_eq!(a.bearer_format.as_deref(), Some("JWT"));
    }

    #[test]
    fn capabilities_become_skills_with_uuid_ids() {
        let card = synthesize_pull_card(
            &agent("alice"),
            &[
                capability("cap-uuid-1", "propose_slots"),
                capability("cap-uuid-2", "confirm_slot"),
            ],
            "https://r",
        );
        assert_eq!(card.skills.len(), 2);
        assert_eq!(card.skills[0].id, "cap-uuid-1");
        assert_eq!(card.skills[0].name, "propose_slots");
        assert_eq!(card.skills[1].id, "cap-uuid-2");
        assert_eq!(card.skills[1].name, "confirm_slot");
    }

    #[test]
    fn empty_description_omits_field_in_serialization() {
        let row = AgentRowForSynthesis {
            account_slug: "x".into(),
            agent_slug: "y".into(),
            display_name: "Y".into(),
            description: "".into(),
        };
        let card = synthesize_pull_card(&row, &[], "https://r");
        assert!(card.description.is_none());
        // Re-parse to check the card-level field specifically (the
        // auth scheme also has a description field that does serialize,
        // so a string-contains check on the whole JSON would be wrong).
        let value: serde_json::Value = serde_json::to_value(&card).unwrap();
        assert!(value.get("description").is_none(), "got: {value}");
    }

    #[test]
    fn skill_with_empty_description_omits_field() {
        let mut cap = capability("c1", "x");
        cap.description = "".into();
        let card = synthesize_pull_card(&agent("alice"), &[cap], "https://r");
        assert!(card.skills[0].description.is_none());
    }

    #[test]
    fn capabilities_streaming_defaults_false() {
        let card = synthesize_pull_card(&agent("alice"), &[], "https://r");
        let caps = card.capabilities.unwrap();
        assert!(!caps.streaming);
        assert!(!caps.push_notifications);
    }

    #[test]
    fn round_trip_through_json() {
        let card = synthesize_pull_card(
            &agent("alice"),
            &[capability("cap-1", "propose_slots")],
            "https://chakramcp.com",
        );
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, card);
    }
}
