//! Pull-mode A2A v0.3 card synthesis.
//!
//! For a pull-mode agent (no public A2A endpoint, runs `inbox.serve()`
//! against our inbox bridge), we have no upstream Agent Card to fetch.
//! Instead we synthesize a card from registration data that is fully
//! wire-compatible with canonical A2A v0.3 — generic A2A clients
//! parse it without ChakraMCP-specific knowledge.
//!
//! What this module produces:
//!
//! - `name`, `description`, `version` from the agents row.
//! - `supported_interfaces` with one entry pointing at our relay
//!   endpoint (`<base>/agents/<account>/<slug>/a2a/jsonrpc`,
//!   binding `JSONRPC`, protocol_version `"0.3"`).
//! - `capabilities` with all flags `false` for v1.
//! - `security_schemes` declaring one scheme (`chakramcp_bearer`,
//!   HTTP+Bearer+JWT) per discovery spec §"Override #4".
//! - `security_requirements` requiring that scheme.
//! - `default_input_modes` / `default_output_modes` defaulted to
//!   `["application/json"]`. Per-skill overrides come from
//!   capability rows if needed.
//! - `skills` from agent_capabilities rows. JSON Schemas do NOT
//!   appear in the card per A2A spec — they live in our REST
//!   capabilities endpoint at
//!   `/v1/discovery/agents/<account>/<slug>/capabilities`.
//!
//! Returned cards are unsigned — `signatures` is empty. The caller
//! (D2c HTTP handler) is expected to invoke `signer::sign_card`
//! before serving.

use std::collections::BTreeMap;

use super::types::{
    AgentCapabilities, AgentCard, AgentInterface, AgentSkill, HttpAuthDetails,
    HttpAuthSecurityScheme, SecurityRequirement, SecurityScheme, A2A_PROTOCOL_VERSION,
    DEFAULT_MEDIA_TYPE, PROTOCOL_BINDING_JSONRPC,
};

/// The canonical scheme name we publish on every card. Keep stable —
/// `security_requirements` references it.
pub const SECURITY_SCHEME_NAME: &str = "chakramcp_bearer";

/// Minimal projection of an `agents` row for synthesis. Constructing
/// this from a sqlx-fetched row keeps the synthesizer free of DB types
/// and easy to unit-test.
#[derive(Debug, Clone)]
pub struct AgentRowForSynthesis {
    pub account_slug: String,
    pub agent_slug: String,
    pub display_name: String,
    pub description: String,
    /// Agent's own semver. ChakraMCP defaults to `"0.1.0"` for newly
    /// registered agents until the operator sets one.
    pub agent_version: String,
}

/// Minimal projection of an `agent_capabilities` row. Carries the
/// capability UUID through as the A2A `skill.id` so policy lookups at
/// call time don't need a name → id round-trip.
#[derive(Debug, Clone)]
pub struct CapabilityRowForSynthesis {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Synthesize an unsigned A2A v0.3 card for a pull-mode agent.
///
/// `relay_base_url` MUST be an absolute http/https URL with no path
/// segment (e.g. `"https://chakramcp.com"`). Returns an error if the
/// base URL is malformed.
///
/// Skills in the output appear sorted by skill `id`, regardless of
/// the input order. This keeps the serialized card byte-stable across
/// renders, which matters for signature stability and edge caching.
pub fn synthesize_pull_card(
    agent: &AgentRowForSynthesis,
    capabilities: &[CapabilityRowForSynthesis],
    relay_base_url: &str,
) -> Result<AgentCard, SynthesisError> {
    let base = validate_relay_base(relay_base_url)?;

    let url = format!(
        "{base}/agents/{}/{}/a2a/jsonrpc",
        agent.account_slug, agent.agent_slug
    );

    let mut sorted_caps: Vec<&CapabilityRowForSynthesis> = capabilities.iter().collect();
    sorted_caps.sort_by(|a, b| a.id.cmp(&b.id));

    let skills: Vec<AgentSkill> = sorted_caps
        .iter()
        .map(|c| AgentSkill {
            id: c.id.clone(),
            name: c.name.clone(),
            // A2A v0.3 says description is REQUIRED. Use the agent's
            // capability description; if blank, fall back to the name
            // so we still emit a non-empty string.
            description: trimmed_or(&c.description, &c.name),
            tags: vec![],
            examples: vec![],
            input_modes: vec![],
            output_modes: vec![],
            security_requirements: vec![],
            extra: Default::default(),
        })
        .collect();

    let mut security_schemes = BTreeMap::new();
    security_schemes.insert(SECURITY_SCHEME_NAME.to_string(), bearer_jwt_scheme());

    let mut chakramcp_req = BTreeMap::new();
    chakramcp_req.insert(SECURITY_SCHEME_NAME.to_string(), Vec::<String>::new());

    Ok(AgentCard {
        name: agent.display_name.clone(),
        // Description is REQUIRED per A2A. Fall back to the display
        // name if the operator left it blank rather than emitting an
        // empty string.
        description: trimmed_or(&agent.description, &agent.display_name),
        supported_interfaces: vec![AgentInterface {
            url,
            protocol_binding: PROTOCOL_BINDING_JSONRPC.to_string(),
            tenant: None,
            protocol_version: A2A_PROTOCOL_VERSION.to_string(),
            extra: Default::default(),
        }],
        provider: None,
        version: agent.agent_version.clone(),
        documentation_url: None,
        capabilities: AgentCapabilities {
            streaming: Some(false),
            push_notifications: Some(false),
            extensions: vec![],
            extended_agent_card: None,
            extra: Default::default(),
        },
        security_schemes,
        security_requirements: vec![SecurityRequirement { schemes: chakramcp_req }],
        default_input_modes: vec![DEFAULT_MEDIA_TYPE.to_string()],
        default_output_modes: vec![DEFAULT_MEDIA_TYPE.to_string()],
        skills,
        signatures: vec![],
        icon_url: None,
        extra: Default::default(),
    })
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SynthesisError {
    #[error("relay_base_url must be a non-empty absolute http(s) URL; got: {0:?}")]
    InvalidBaseUrl(String),
}

/// Validate + normalize the relay base. Strips at most one trailing
/// `/`. Rejects empty strings and non-http(s) bases.
fn validate_relay_base(input: &str) -> Result<String, SynthesisError> {
    let trimmed = input.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(SynthesisError::InvalidBaseUrl(input.to_string()));
    }
    // Reject base URLs that are JUST a scheme (e.g., "https://").
    let after_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    if after_scheme.is_empty() || after_scheme.starts_with('/') {
        return Err(SynthesisError::InvalidBaseUrl(input.to_string()));
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// `s.trim()` if non-empty after trimming; otherwise `fallback`.
fn trimmed_or(s: &str, fallback: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        fallback.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// The single security scheme ChakraMCP publishes on every card.
fn bearer_jwt_scheme() -> SecurityScheme {
    SecurityScheme::Http(HttpAuthSecurityScheme {
        http: HttpAuthDetails {
            description: Some(
                "ChakraMCP-issued bearer token (API key or OAuth-issued JWT).".to_string(),
            ),
            scheme: "Bearer".to_string(),
            bearer_format: Some("JWT".to_string()),
            extra: Default::default(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(slug: &str) -> AgentRowForSynthesis {
        AgentRowForSynthesis {
            account_slug: "acme-corp".into(),
            agent_slug: slug.into(),
            display_name: "Alice Scheduler".into(),
            description: "Returns 30-min slots in the next N days.".into(),
            agent_version: "0.1.0".into(),
        }
    }

    fn cap(id: &str, name: &str) -> CapabilityRowForSynthesis {
        CapabilityRowForSynthesis {
            id: id.into(),
            name: name.into(),
            description: "Plan a thing.".into(),
        }
    }

    #[test]
    fn url_uses_path_based_relay_endpoint() {
        let card =
            synthesize_pull_card(&agent("alice"), &[], "https://chakramcp.com").unwrap();
        assert_eq!(card.supported_interfaces.len(), 1);
        assert_eq!(
            card.supported_interfaces[0].url,
            "https://chakramcp.com/agents/acme-corp/alice/a2a/jsonrpc"
        );
        assert_eq!(card.supported_interfaces[0].protocol_binding, "JSONRPC");
        assert_eq!(card.supported_interfaces[0].protocol_version, "0.3");
    }

    #[test]
    fn trailing_slash_on_base_is_normalized() {
        let card =
            synthesize_pull_card(&agent("alice"), &[], "https://chakramcp.com/").unwrap();
        assert_eq!(
            card.supported_interfaces[0].url,
            "https://chakramcp.com/agents/acme-corp/alice/a2a/jsonrpc"
        );
    }

    #[test]
    fn rejects_empty_base() {
        assert_eq!(
            synthesize_pull_card(&agent("a"), &[], ""),
            Err(SynthesisError::InvalidBaseUrl("".to_string()))
        );
    }

    #[test]
    fn rejects_relative_base() {
        let r = synthesize_pull_card(&agent("a"), &[], "/relay");
        assert!(matches!(r, Err(SynthesisError::InvalidBaseUrl(_))));
    }

    #[test]
    fn rejects_scheme_only_base() {
        let r = synthesize_pull_card(&agent("a"), &[], "https://");
        assert!(matches!(r, Err(SynthesisError::InvalidBaseUrl(_))));
        let r = synthesize_pull_card(&agent("a"), &[], "https:///path");
        assert!(matches!(r, Err(SynthesisError::InvalidBaseUrl(_))));
    }

    #[test]
    fn rejects_non_http_scheme() {
        let r = synthesize_pull_card(&agent("a"), &[], "ftp://x");
        assert!(matches!(r, Err(SynthesisError::InvalidBaseUrl(_))));
    }

    #[test]
    fn always_bearer_jwt_only() {
        let card = synthesize_pull_card(&agent("alice"), &[], "https://r/x").unwrap();
        assert_eq!(card.security_schemes.len(), 1);
        let scheme = card.security_schemes.get("chakramcp_bearer").unwrap();
        let SecurityScheme::Http(h) = scheme else {
            panic!("expected http scheme");
        };
        assert_eq!(h.http.scheme, "Bearer");
        assert_eq!(h.http.bearer_format.as_deref(), Some("JWT"));
        // And the requirement points at it.
        assert_eq!(card.security_requirements.len(), 1);
        assert!(card.security_requirements[0]
            .schemes
            .contains_key("chakramcp_bearer"));
    }

    #[test]
    fn capabilities_become_skills_with_uuid_ids_sorted() {
        let card = synthesize_pull_card(
            &agent("alice"),
            // Out-of-order input: synthesizer should sort by id.
            &[
                cap("cap-uuid-2", "confirm_slot"),
                cap("cap-uuid-1", "propose_slots"),
                cap("cap-uuid-3", "cancel"),
            ],
            "https://r/x",
        )
        .unwrap();
        assert_eq!(card.skills.len(), 3);
        assert_eq!(card.skills[0].id, "cap-uuid-1");
        assert_eq!(card.skills[1].id, "cap-uuid-2");
        assert_eq!(card.skills[2].id, "cap-uuid-3");
    }

    #[test]
    fn skill_includes_required_fields() {
        let card = synthesize_pull_card(
            &agent("alice"),
            &[cap("cap-uuid-1", "propose_slots")],
            "https://r/x",
        )
        .unwrap();
        let skill = &card.skills[0];
        assert_eq!(skill.id, "cap-uuid-1");
        assert_eq!(skill.name, "propose_slots");
        assert_eq!(skill.description, "Plan a thing.");
        // tags is REQUIRED in spec; an empty Vec is acceptable. We
        // skip_serializing_if empty so the JSON omits it; on parse the
        // default impl reconstructs an empty Vec. Round-trip preserves
        // the (empty) value.
        let json = serde_json::to_string(skill).unwrap();
        let parsed: AgentSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tags, Vec::<String>::new());
        assert_eq!(parsed.examples, Vec::<String>::new());
        assert_eq!(parsed.input_modes, Vec::<String>::new());
    }

    #[test]
    fn slug_must_match_charset_assumption() {
        // The synthesizer trusts the registration layer to enforce
        // `[a-z0-9-]+` slugs. If that contract is ever broken, the URL
        // composition emits a malformed URL. This test pins down the
        // canonical-charset behavior so a regression is caught here.
        let card = synthesize_pull_card(
            &AgentRowForSynthesis {
                account_slug: "acme-corp".into(),
                agent_slug: "alice-v2".into(),
                display_name: "X".into(),
                description: "Y".into(),
                agent_version: "0.1.0".into(),
            },
            &[],
            "https://chakramcp.com",
        )
        .unwrap();
        // Hyphens and digits round-trip without encoding; if anyone
        // changes the slug regex to allow `/` or spaces, this test
        // doesn't catch it directly — see registration-layer tests.
        assert_eq!(
            card.supported_interfaces[0].url,
            "https://chakramcp.com/agents/acme-corp/alice-v2/a2a/jsonrpc"
        );
    }

    use super::super::types::AgentSkill;

    #[test]
    fn empty_description_falls_back_to_display_name() {
        let mut row = agent("alice");
        row.description = "   ".to_string(); // whitespace-only
        let card = synthesize_pull_card(&row, &[], "https://r/x").unwrap();
        // A2A description is REQUIRED; fallback prevents empty string.
        assert_eq!(card.description, "Alice Scheduler");
    }

    #[test]
    fn empty_capability_description_falls_back_to_name() {
        let mut c = cap("c1", "summarize");
        c.description = "".to_string();
        let card = synthesize_pull_card(&agent("alice"), &[c], "https://r/x").unwrap();
        assert_eq!(card.skills[0].description, "summarize");
    }

    #[test]
    fn unicode_name_round_trips() {
        let mut row = agent("alice");
        row.display_name = "アリス Scheduler 🗓️".into();
        let card = synthesize_pull_card(&row, &[], "https://r/x").unwrap();
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "アリス Scheduler 🗓️");
    }

    #[test]
    fn determinism_two_orderings_produce_byte_equal_json() {
        let card_a = synthesize_pull_card(
            &agent("alice"),
            &[cap("c1", "a"), cap("c2", "b"), cap("c3", "c")],
            "https://r/x",
        )
        .unwrap();
        let card_b = synthesize_pull_card(
            &agent("alice"),
            &[cap("c3", "c"), cap("c1", "a"), cap("c2", "b")],
            "https://r/x",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&card_a).unwrap(),
            serde_json::to_string(&card_b).unwrap()
        );
    }

    #[test]
    fn round_trip_through_json() {
        let card = synthesize_pull_card(
            &agent("alice"),
            &[cap("cap-1", "propose_slots")],
            "https://chakramcp.com",
        )
        .unwrap();
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, card);
    }

    #[test]
    fn defaults_capabilities_streaming_false() {
        let card = synthesize_pull_card(&agent("alice"), &[], "https://r/x").unwrap();
        assert_eq!(card.capabilities.streaming, Some(false));
        assert_eq!(card.capabilities.push_notifications, Some(false));
    }

    #[test]
    fn default_modes_are_application_json() {
        let card = synthesize_pull_card(&agent("alice"), &[], "https://r/x").unwrap();
        assert_eq!(card.default_input_modes, vec!["application/json"]);
        assert_eq!(card.default_output_modes, vec!["application/json"]);
    }
}
