//! Canonical A2A v0.3 Agent Card types.
//!
//! Modelled directly from the protobuf schema at
//! <https://github.com/a2aproject/A2A/blob/main/specification/a2a.proto>.
//! Field names, casing, plurality, and required-vs-optional are all
//! aligned to the spec so generic A2A clients (Google's reference SDK,
//! openclaw-a2a-gateway, future implementations) can parse cards we
//! publish without any ChakraMCP-specific knowledge.
//!
//! ChakraMCP-specific data (capability JSON Schemas, friendship state,
//! grant policy) lives in our own REST endpoints
//! (`/v1/discovery/agents/<account>/<slug>/capabilities`), not in the
//! card. The card is a lean A2A discovery descriptor.
//!
//! Forward compatibility: every struct flattens into an `extra` map of
//! unknown fields. Parsing an A2A v1.x card (or any third-party
//! extension) and re-publishing it is lossless — fields we don't model
//! survive the round-trip via `serde(flatten)`.
//!
//! Signing: `signatures` is plural per spec. Each entry is JWS-shaped
//! per RFC 7515 (`protected` header + `signature` value, both
//! base64url). The signing implementation lives in `signer.rs` (D2b)
//! and operates on a canonical-JSON projection of the card; multiple
//! signatures for the same card are explicitly allowed by the spec
//! (e.g. one ChakraMCP signature plus a re-publish signature from a
//! mirror).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Canonical A2A protocol version this implementation targets.
/// Used as the value for `AgentInterface.protocol_version` on every
/// interface we publish.
pub const A2A_PROTOCOL_VERSION: &str = "0.3";

/// JSON-RPC binding identifier per A2A spec.
/// `AgentInterface.protocol_binding` for the JSON-RPC interface.
pub const PROTOCOL_BINDING_JSONRPC: &str = "JSONRPC";

/// Default media type for ChakraMCP-mediated traffic. Capabilities
/// can override per-skill via `AgentSkill.input_modes` /
/// `output_modes`.
pub const DEFAULT_MEDIA_TYPE: &str = "application/json";

/// Top-level Agent Card published at
/// `/.well-known/agent-card.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCard {
    /// Human-readable agent name. REQUIRED.
    pub name: String,

    /// Human-readable description, one to two sentences. REQUIRED.
    pub description: String,

    /// Ordered list of supported interfaces; first entry is preferred.
    /// REQUIRED, must contain at least one entry. ChakraMCP publishes
    /// exactly one entry that points at our relay.
    pub supported_interfaces: Vec<AgentInterface>,

    /// Service provider metadata. Optional per spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,

    /// Agent's own semver. REQUIRED. NOT the A2A protocol version —
    /// that lives in `AgentInterface.protocol_version`. Example:
    /// `"0.1.0"`.
    pub version: String,

    /// Optional URL pointing at human/agent-readable docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,

    /// Coarse capability flags. REQUIRED per spec (we always emit it,
    /// even with all flags `false`).
    pub capabilities: AgentCapabilities,

    /// Map from scheme name → typed security scheme. ChakraMCP always
    /// emits one entry, `chakramcp_bearer`, of type `http+Bearer+JWT`.
    /// Optional per spec.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub security_schemes: BTreeMap<String, SecurityScheme>,

    /// Which schemes a caller MUST satisfy. ChakraMCP requires the
    /// `chakramcp_bearer` scheme. Optional per spec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_requirements: Vec<SecurityRequirement>,

    /// Media types accepted as input across all skills. REQUIRED.
    /// Per-skill values in `AgentSkill.input_modes` override.
    pub default_input_modes: Vec<String>,

    /// Media types produced as output across all skills. REQUIRED.
    pub default_output_modes: Vec<String>,

    /// Skills (capabilities) the agent exposes. REQUIRED, can be
    /// empty for an agent that exists only to receive trust events.
    pub skills: Vec<AgentSkill>,

    /// JWS signatures. Optional per spec; ChakraMCP always emits at
    /// least one entry on cards it publishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<AgentCardSignature>,

    /// Optional URL to the agent's icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,

    /// Forward-compat: any spec-extension or future fields we don't
    /// model live here. Survives round-trip serialization.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One callable interface to the agent. ChakraMCP publishes exactly
/// one entry pointing at our relay endpoint, so all A2A traffic
/// flows through our policy proxy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInterface {
    /// REQUIRED. Absolute URL of the A2A endpoint. ChakraMCP emits
    /// `<relay-base>/agents/<account>/<slug>/a2a/jsonrpc`.
    pub url: String,

    /// REQUIRED. Open-form binding identifier. Spec lists
    /// `JSONRPC`, `GRPC`, `HTTP+JSON`. ChakraMCP emits `JSONRPC`.
    pub protocol_binding: String,

    /// Tenant identifier. Optional. Unused by ChakraMCP (account is
    /// already encoded in the URL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,

    /// REQUIRED. Protocol version this interface speaks. ChakraMCP
    /// emits `"0.3"` (see [`A2A_PROTOCOL_VERSION`]).
    pub protocol_version: String,

    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Agent provider — who runs the agent. Optional. ChakraMCP can
/// populate this from the owning account's display info.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentProvider {
    pub organization: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A skill is a named operation the agent can perform.
///
/// Per A2A spec, skills do NOT carry JSON Schemas inline — they only
/// describe *modes* (media types) and *examples* (free-text prompts /
/// scenarios). ChakraMCP exposes input/output JSON Schemas via our own
/// REST endpoint at
/// `/v1/discovery/agents/<account>/<slug>/capabilities`. The card
/// remains a lean discovery descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSkill {
    /// REQUIRED. Stable identifier within the agent. ChakraMCP uses
    /// the capability UUID so policy lookups at call time don't need
    /// a name → id round-trip.
    pub id: String,

    /// REQUIRED. Human-readable name (often the same as `id`).
    pub name: String,

    /// REQUIRED. Detailed description.
    pub description: String,

    /// REQUIRED per A2A v0.3 (proto field 4). Keywords describing the
    /// skill's capabilities. Empty Vec is acceptable.
    pub tags: Vec<String>,

    /// Optional. Free-form example prompts / scenarios.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,

    /// Optional. Override agent default input modes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,

    /// Optional. Override agent default output modes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,

    /// Optional. Override agent default security requirements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_requirements: Vec<SecurityRequirement>,

    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Coarse capability flags + protocol extensions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<AgentExtension>,

    /// True if the agent serves a richer card to authenticated callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_agent_card: Option<bool>,

    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Protocol extension declaration. Open-ended — extension schemas are
/// defined by their authors and identified by URI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentExtension {
    /// URI identifying the extension.
    pub uri: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the agent requires the extension for normal operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Extension-specific parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// JWS-shaped signature per RFC 7515. The `protected` header and
/// `signature` are base64url-encoded; the signature covers a
/// canonical-JSON projection of the card body (the projection rules
/// live in `signer.rs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCardSignature {
    /// REQUIRED. Base64url-encoded JWS protected header (JSON object
    /// with `alg`, `kid`, etc.).
    pub protected: String,

    /// REQUIRED. Base64url-encoded signature bytes.
    pub signature: String,

    /// Unprotected header fields. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<serde_json::Value>,

    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

// ─────────────────────────────────────────────────────────
// Security: a oneof in the proto, modeled here as an internally-tagged
// enum keyed by the variant field name. Matches protobuf-to-JSON
// canonical form: `{ "http": { ... } }`.
// ─────────────────────────────────────────────────────────

/// One of the typed security schemes. ChakraMCP always emits the
/// `http` variant (Bearer + JWT) but other variants exist for
/// completeness and forward parsing of upstream cards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SecurityScheme {
    Http(HttpAuthSecurityScheme),
    ApiKey(ApiKeySecurityScheme),
    OAuth2(OAuth2SecurityScheme),
    OpenIdConnect(OpenIdConnectSecurityScheme),
    MutualTls(MutualTlsSecurityScheme),
    /// Forward compat for any future scheme types.
    Other(serde_json::Map<String, serde_json::Value>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpAuthSecurityScheme {
    /// The "http" key tags this variant; payload follows.
    pub http: HttpAuthDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpAuthDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// REQUIRED. RFC 7235 scheme name, e.g. `"Bearer"` or `"Basic"`.
    pub scheme: String,
    /// Optional bearer-token format hint, e.g. `"JWT"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_format: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeySecurityScheme {
    pub api_key: ApiKeyDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `header` | `query` | `cookie` per spec.
    #[serde(rename = "in")]
    pub location: String,
    pub name: String,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuth2SecurityScheme {
    pub oauth2: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenIdConnectSecurityScheme {
    pub open_id_connect: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MutualTlsSecurityScheme {
    pub mutual_tls: serde_json::Value,
}

/// Map from scheme name → list of required scopes (empty for non-
/// OAuth2 schemes). Multiple `SecurityRequirement` entries in the
/// list are OR'd; multiple keys within a single entry are AND'd.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityRequirement {
    /// Each entry: scheme name → required scopes (empty Vec for
    /// non-OAuth schemes like our HTTP+Bearer).
    #[serde(flatten)]
    pub schemes: BTreeMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn minimal_required_card_round_trips() {
        let card = AgentCard {
            name: "Alice".to_string(),
            description: "Demo agent.".to_string(),
            supported_interfaces: vec![AgentInterface {
                url: "https://r/x".to_string(),
                protocol_binding: PROTOCOL_BINDING_JSONRPC.to_string(),
                tenant: None,
                protocol_version: A2A_PROTOCOL_VERSION.to_string(),
                extra: Default::default(),
            }],
            provider: None,
            version: "0.1.0".to_string(),
            documentation_url: None,
            capabilities: AgentCapabilities::default(),
            security_schemes: BTreeMap::new(),
            security_requirements: vec![],
            default_input_modes: vec!["application/json".to_string()],
            default_output_modes: vec!["application/json".to_string()],
            skills: vec![],
            signatures: vec![],
            icon_url: None,
            extra: Default::default(),
        };
        let json = serde_json::to_string(&card).unwrap();
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, card);
    }

    #[test]
    fn unknown_top_level_fields_survive_round_trip() {
        let json = json!({
            "name": "X",
            "description": "Y",
            "supported_interfaces": [{
                "url": "https://r/x",
                "protocol_binding": "JSONRPC",
                "protocol_version": "0.3"
            }],
            "version": "0.1.0",
            "capabilities": {},
            "default_input_modes": ["application/json"],
            "default_output_modes": ["application/json"],
            "skills": [],
            "future_field_added_in_v1_2": {"new": "data"},
            "another_unknown": 42
        });
        let card: AgentCard = serde_json::from_value(json.clone()).unwrap();
        let reserialized = serde_json::to_value(&card).unwrap();
        assert_eq!(reserialized.get("future_field_added_in_v1_2"),
                   json.get("future_field_added_in_v1_2"));
        assert_eq!(reserialized.get("another_unknown"),
                   json.get("another_unknown"));
    }

    #[test]
    fn http_bearer_security_scheme_serializes_canonically() {
        let scheme = SecurityScheme::Http(HttpAuthSecurityScheme {
            http: HttpAuthDetails {
                description: Some("ChakraMCP-issued bearer.".to_string()),
                scheme: "Bearer".to_string(),
                bearer_format: Some("JWT".to_string()),
                extra: Default::default(),
            },
        });
        let json = serde_json::to_value(&scheme).unwrap();
        // Untagged enum + nested struct produces { "http": { scheme, bearer_format, ... } }.
        assert_eq!(json["http"]["scheme"], "Bearer");
        assert_eq!(json["http"]["bearer_format"], "JWT");
        // Round-trip.
        let parsed: SecurityScheme = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, scheme);
    }

    #[test]
    fn security_requirement_serializes_as_flat_map() {
        let mut schemes = BTreeMap::new();
        schemes.insert("chakramcp_bearer".to_string(), Vec::<String>::new());
        let req = SecurityRequirement { schemes };
        let json = serde_json::to_value(&req).unwrap();
        // Should serialize as { "chakramcp_bearer": [] }, not
        // { "schemes": { "chakramcp_bearer": [] } }.
        assert!(json.get("chakramcp_bearer").is_some());
        assert!(json.get("schemes").is_none());
    }

    #[test]
    fn signature_is_jws_shaped() {
        let sig = AgentCardSignature {
            protected: "eyJhbGciOiJFZERTQSIsImtpZCI6InJlbGF5LTIwMjYtMDQifQ".to_string(),
            signature: "yKQ7iQuy1Vqd1J47w-FbFUXDWDsEgpr9xPEEEMo_RZBmvg9w".to_string(),
            header: None,
            extra: Default::default(),
        };
        let json = serde_json::to_value(&sig).unwrap();
        assert!(json["protected"].is_string());
        assert!(json["signature"].is_string());
        assert!(json.get("alg").is_none(), "JWS shape, not custom alg field");
        assert!(json.get("kid").is_none(), "kid is inside protected header");
    }

    #[test]
    fn skills_use_canonical_field_names() {
        let skill = AgentSkill {
            id: "uuid-1".to_string(),
            name: "summarize".to_string(),
            description: "Summarize text.".to_string(),
            tags: vec!["nlp".to_string()],
            examples: vec!["Summarize this paragraph.".to_string()],
            input_modes: vec!["text/plain".to_string()],
            output_modes: vec!["text/plain".to_string()],
            security_requirements: vec![],
            extra: Default::default(),
        };
        let json = serde_json::to_string(&skill).unwrap();
        // Spec uses snake_case, NOT inputSchema / outputSchema.
        assert!(json.contains(r#""input_modes":"#));
        assert!(json.contains(r#""output_modes":"#));
        assert!(!json.contains("inputSchema"));
        assert!(!json.contains("outputSchema"));
        let parsed: AgentSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, skill);
    }

    #[test]
    fn multiple_signatures_round_trip() {
        // Spec allows multiple signatures (e.g. our wrap + an upstream
        // canonical signature). Verify both survive parse + reserialize.
        let json = json!({
            "name": "X",
            "description": "Y",
            "supported_interfaces": [{
                "url": "https://r/x",
                "protocol_binding": "JSONRPC",
                "protocol_version": "0.3"
            }],
            "version": "0.1.0",
            "capabilities": {},
            "default_input_modes": ["application/json"],
            "default_output_modes": ["application/json"],
            "skills": [],
            "signatures": [
                { "protected": "header-relay", "signature": "sig-relay" },
                { "protected": "header-upstream", "signature": "sig-upstream",
                  "header": { "kid": "upstream-2026-01" } }
            ]
        });
        let card: AgentCard = serde_json::from_value(json).unwrap();
        assert_eq!(card.signatures.len(), 2);
        assert_eq!(card.signatures[0].signature, "sig-relay");
        assert_eq!(card.signatures[1].header.as_ref().unwrap()["kid"], "upstream-2026-01");
        let reserialized = serde_json::to_value(&card).unwrap();
        let reparsed: AgentCard = serde_json::from_value(reserialized).unwrap();
        assert_eq!(card, reparsed);
    }

    #[test]
    fn unknown_fields_inside_skill_and_interface_survive_round_trip() {
        // Forward-compat: extension fields under nested structs (not just
        // top-level) must round-trip. A2A v1.x is likely to add fields here.
        let json = json!({
            "name": "X",
            "description": "Y",
            "supported_interfaces": [{
                "url": "https://r/x",
                "protocol_binding": "JSONRPC",
                "protocol_version": "0.3",
                "future_interface_field": "mtls-config"
            }],
            "version": "0.1.0",
            "capabilities": {
                "future_capability_flag": true
            },
            "default_input_modes": ["application/json"],
            "default_output_modes": ["application/json"],
            "skills": [{
                "id": "x", "name": "x", "description": "x", "tags": [],
                "future_skill_field": { "rate_limit": "10/s" }
            }]
        });
        let card: AgentCard = serde_json::from_value(json.clone()).unwrap();
        let reserialized = serde_json::to_value(&card).unwrap();
        assert_eq!(reserialized["supported_interfaces"][0]["future_interface_field"],
                   json["supported_interfaces"][0]["future_interface_field"]);
        assert_eq!(reserialized["capabilities"]["future_capability_flag"],
                   json["capabilities"]["future_capability_flag"]);
        assert_eq!(reserialized["skills"][0]["future_skill_field"],
                   json["skills"][0]["future_skill_field"]);
    }

    #[test]
    fn parse_canonical_a2a_v03_example() {
        // Modeled after the A2A spec's "Recipe Agent" example. If
        // generic A2A clients can parse this, our types are wire-compat.
        let example = json!({
            "name": "Recipe Agent",
            "description": "Agent that helps users with recipes and cooking.",
            "supported_interfaces": [
                {
                    "url": "https://api.example.com/a2a/v1",
                    "protocol_binding": "JSONRPC",
                    "protocol_version": "0.3"
                }
            ],
            "provider": {
                "organization": "Example Foods Inc.",
                "url": "https://example.com"
            },
            "version": "1.2.3",
            "documentation_url": "https://example.com/recipe-agent/docs",
            "capabilities": {
                "streaming": true,
                "push_notifications": false
            },
            "security_schemes": {
                "api_key_header": {
                    "api_key": {
                        "in": "header",
                        "name": "X-API-Key"
                    }
                }
            },
            "security_requirements": [
                { "api_key_header": [] }
            ],
            "default_input_modes": ["application/json", "text/plain"],
            "default_output_modes": ["application/json"],
            "skills": [
                {
                    "id": "recipe-search",
                    "name": "Recipe Search",
                    "description": "Find recipes by ingredient or cuisine.",
                    "tags": ["recipes", "search"],
                    "examples": ["What can I make with chicken and rice?"],
                    "input_modes": ["text/plain"],
                    "output_modes": ["application/json"]
                }
            ],
            "icon_url": "https://example.com/icon.png"
        });
        let card: AgentCard = serde_json::from_value(example.clone()).unwrap();
        assert_eq!(card.name, "Recipe Agent");
        assert_eq!(card.version, "1.2.3");
        assert_eq!(card.supported_interfaces[0].url, "https://api.example.com/a2a/v1");
        assert_eq!(card.supported_interfaces[0].protocol_version, "0.3");
        assert_eq!(card.skills[0].id, "recipe-search");
        assert_eq!(card.skills[0].tags, vec!["recipes".to_string(), "search".to_string()]);
        // Round-trip to JSON and reparse equal.
        let reserialized = serde_json::to_value(&card).unwrap();
        let reparsed: AgentCard = serde_json::from_value(reserialized).unwrap();
        assert_eq!(card, reparsed);
    }
}
