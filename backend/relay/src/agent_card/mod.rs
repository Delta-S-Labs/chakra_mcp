//! Canonical A2A v0.3 Agent Card service.
//!
//! ChakraMCP publishes the canonical, public-facing A2A Agent Card for every
//! opted-in registered agent at
//! `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json`.
//!
//! Two derivation paths feed the published card body:
//!
//! - **Push** — the agent has a public A2A endpoint declared in
//!   `agents.agent_card_url`. The relay fetches the upstream card,
//!   substitutes our relay URL into the `supported_interfaces[].url`
//!   so all traffic hits us first for policy enforcement, re-signs,
//!   caches.
//!
//! - **Pull** — the agent has no public host and runs `inbox.serve()`
//!   against our inbox bridge. The relay synthesizes a card from
//!   registration data (display name, description, capability rows).
//!
//! In both cases the card we publish lists *our* relay endpoint as the
//! `supported_interfaces[0].url`, never the agent's actual A2A
//! endpoint. The URL is in the JWS-protected projection, so signed-card
//! replay with substituted URLs fails verification.
//!
//! Type definitions match the A2A v0.3 protobuf schema at
//! <https://github.com/a2aproject/A2A/blob/main/specification/a2a.proto>.
//! Generic A2A clients (Google's reference SDK, openclaw-a2a-gateway,
//! future implementations) parse our cards without ChakraMCP-specific
//! knowledge. Forward compatibility for newer A2A versions is via
//! `serde(flatten)` on every struct so unknown fields survive parse +
//! re-serialize.
//!
//! ChakraMCP-specific data (capability JSON Schemas, friendship state,
//! grant policy) lives in our own REST endpoints
//! (`/v1/discovery/agents/<account>/<slug>/capabilities`), not in the
//! card.
//!
//! See `docs/specs/2026-04-29-discovery-design.md` §"Agent Card hosting
//! model" for the full design.

pub mod fetcher;
pub mod keys;
pub mod signer;
pub mod synthesizer;
pub mod types;

pub use fetcher::{
    cache_card_for_agent, normalize_for_publish, CacheError, CachedCardEnvelope, FetchError,
    FetchOutcome, Fetcher, MAX_REFRESH_INTERVAL_SECONDS,
};
pub use keys::{Jwks, JsonWebKey, KeyStore, KeyStoreError};
pub use signer::{sign_card, verify_card, SignError, SigningKey, VerifyError, VerifyingKey};
pub use synthesizer::{
    synthesize_pull_card, AgentRowForSynthesis, CapabilityRowForSynthesis, SynthesisError,
    SECURITY_SCHEME_NAME,
};
pub use types::{
    AgentCapabilities, AgentCard, AgentCardSignature, AgentExtension, AgentInterface,
    AgentProvider, AgentSkill, ApiKeyDetails, ApiKeySecurityScheme, HttpAuthDetails,
    HttpAuthSecurityScheme, MutualTlsSecurityScheme, OAuth2SecurityScheme,
    OpenIdConnectSecurityScheme, SecurityRequirement, SecurityScheme,
    A2A_PROTOCOL_VERSION, DEFAULT_MEDIA_TYPE, PROTOCOL_BINDING_JSONRPC,
};
