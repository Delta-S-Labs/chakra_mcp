//! A2A Agent Card types.
//!
//! Mirrors the structure documented in the A2A v0.3+/v1.0 protocol spec
//! (https://a2a-protocol.org/latest/specification/) with two
//! ChakraMCP-specific notes:
//!
//! 1. The published card's `authentication` always declares
//!    `[ http+bearer ]` — see discovery spec §"Override #4". Upstream
//!    cards declaring other schemes get normalized on republish.
//!
//! 2. The `signature` field carries our Ed25519 signature with an
//!    explicit `covered_fields` list. We sign the card we publish, not
//!    the upstream original. If an upstream card was itself signed,
//!    that signature is preserved in `original_signature` for audit
//!    but does not appear at the top level of the published card.
//!
//! The structs derive `Serialize` + `Deserialize` so the same types
//! cover (a) parsing upstream push cards we fetch, (b) building our
//! synthesized published cards, (c) re-serializing for HTTP responses.

use serde::{Deserialize, Serialize};

/// Top-level Agent Card published at `/.well-known/agent-card.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentCard {
    /// Display name. Free-form, human-readable.
    pub name: String,

    /// Short description. One sentence ideally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Canonical A2A JSON-RPC endpoint. **Always our relay URL**, never
    /// the upstream agent's endpoint, for in-path policy enforcement.
    pub url: String,

    /// Card schema/protocol version this conforms to. Use `"0.3"`
    /// while we're targeting A2A v0.3 wire format.
    pub version: String,

    /// What the agent can do, declared in spec terms. ChakraMCP layers
    /// its own policy overlay on top via `agents.tags` /
    /// per-capability visibility (see discovery spec).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub skills: Vec<AgentSkill>,

    /// Auth schemes the caller must use. ChakraMCP always declares
    /// `[ http+bearer (JWT) ]` in v1; A2A clients fetch a token from
    /// our relay (or use a ChakraMCP API key) and present it as a
    /// Bearer header on the actual A2A method call.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub authentication: Vec<AgentAuthentication>,

    /// Coarse capability flags. Whether the agent supports streaming,
    /// push notifications, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<AgentCapabilities>,

    /// ChakraMCP signature over a canonical projection of this card.
    /// Required on every card we publish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<AgentSignature>,

    /// If the upstream canonical card carried its own signature
    /// (A2A v1.0+), preserve it here for audit. Generic A2A clients
    /// should verify the top-level `signature` (ours), not this one.
    #[serde(skip_serializing_if = "Option::is_none", rename = "originalSignature")]
    pub original_signature: Option<serde_json::Value>,
}

/// A skill is a named operation the agent can perform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSkill {
    /// Stable identifier within the agent. We carry our internal
    /// capability UUID through here so policy lookups don't require a
    /// secondary name → id mapping at call time.
    pub id: String,

    /// Human-readable name. Often equal to `id` for terse cases.
    pub name: String,

    /// One-sentence description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// JSON Schema for the input message. Empty `{}` means
    /// "schemaless" — accepts anything.
    #[serde(rename = "inputSchema")]
    pub input_schema: JsonSchema,

    /// JSON Schema for the output message.
    #[serde(rename = "outputSchema")]
    pub output_schema: JsonSchema,
}

/// Auth scheme declaration. ChakraMCP only ever publishes
/// `http + bearer + JWT`; richer schemes live behind future feature
/// flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentAuthentication {
    /// `"http"` for HTTP auth, `"apiKey"` for header-based API keys,
    /// `"oauth2"` / `"openIdConnect"` for those flows. ChakraMCP
    /// always emits `"http"` in v1.
    pub scheme: String,

    /// `"bearer"` / `"basic"` / etc. ChakraMCP always emits `"bearer"`.
    #[serde(rename = "type")]
    pub auth_type: String,

    /// `"JWT"` is the format we issue. Generic A2A clients can also
    /// present ChakraMCP API keys (`ck_…`) as the bearer; both work.
    #[serde(rename = "bearerFormat", skip_serializing_if = "Option::is_none")]
    pub bearer_format: Option<String>,

    /// Free-form note for humans / generic A2A client docs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Coarse capability flags. Empty struct is fine — defaults to all
/// false unless explicitly set true.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentCapabilities {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,

    #[serde(default, skip_serializing_if = "is_false", rename = "pushNotifications")]
    pub push_notifications: bool,
}

/// Ed25519 signature over a canonical JSON projection of the card's
/// covered fields. See `signer.rs` (D2b) for serialization rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSignature {
    /// Signature algorithm. ChakraMCP always uses `"EdDSA"`.
    pub alg: String,

    /// Key ID — references a public key entry in our JWKS at
    /// `chakramcp.com/.well-known/jwks.json`. Rotated on a 90-day
    /// cycle with a 30-day overlap window.
    pub kid: String,

    /// Field names included in the signed projection, in canonical
    /// order. Verifiers reconstruct the projection by reading these
    /// fields from the card and re-serializing them in this order.
    /// Always includes `url`, so signed-card replay with substituted
    /// URLs fails verification.
    #[serde(rename = "covered_fields")]
    pub covered_fields: Vec<String>,

    /// Base64-URL-encoded signature bytes (no padding, per RFC 7515).
    pub value: String,
}

/// JSON Schema is an arbitrary JSON object. We store and pass through
/// without parsing — the agent (or ChakraMCP's own validators when
/// we add one) is responsible for schema semantics.
pub type JsonSchema = serde_json::Value;

fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_authentication_omits_field() {
        let card = AgentCard {
            name: "x".into(),
            description: None,
            url: "https://r/x".into(),
            version: "0.3".into(),
            skills: vec![],
            authentication: vec![],
            capabilities: None,
            signature: None,
            original_signature: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        assert!(!json.contains("authentication"), "got: {json}");
        assert!(!json.contains("skills"), "got: {json}");
    }

    #[test]
    fn capabilities_omit_false_flags() {
        let caps = AgentCapabilities {
            streaming: true,
            push_notifications: false,
        };
        let json = serde_json::to_string(&caps).unwrap();
        // Only the true field appears.
        assert_eq!(json, r#"{"streaming":true}"#);
    }

    #[test]
    fn auth_renames() {
        let auth = AgentAuthentication {
            scheme: "http".into(),
            auth_type: "bearer".into(),
            bearer_format: Some("JWT".into()),
            description: None,
        };
        let json = serde_json::to_string(&auth).unwrap();
        assert!(json.contains(r#""type":"bearer""#));
        assert!(json.contains(r#""bearerFormat":"JWT""#));
        // Round-trip parses cleanly.
        let parsed: AgentAuthentication = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, auth);
    }

    #[test]
    fn skill_input_output_schema_renames() {
        let skill = AgentSkill {
            id: "summarize".into(),
            name: "summarize".into(),
            description: Some("Summarize a block of text.".into()),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&skill).unwrap();
        assert!(json.contains(r#""inputSchema":"#));
        assert!(json.contains(r#""outputSchema":"#));
        let parsed: AgentSkill = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, skill);
    }
}
