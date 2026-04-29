//! Agent Card service.
//!
//! ChakraMCP publishes the canonical, public-facing A2A Agent Card for every
//! opted-in registered agent at
//! `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json`.
//!
//! Two derivation paths feed the published card body:
//!
//! - **Push** — the agent has a public A2A endpoint declared in
//!   `agents.agent_card_url`. The relay fetches the upstream card,
//!   substitutes our relay URL into the `url` field (so all traffic
//!   hits us first for policy enforcement), re-signs, caches.
//!
//! - **Pull** — the agent has no public host and runs `inbox.serve()`
//!   against our inbox bridge. The relay synthesizes a card from
//!   registration data (display name, description, capability rows).
//!
//! In both cases the card we publish lists *our* relay endpoint as the
//! `url`, never the agent's actual A2A endpoint. The `url` field is in
//! the signed-fields scope, so signed-card replay with substituted URLs
//! fails verification.
//!
//! See `docs/specs/2026-04-29-discovery-design.md` §"Agent Card hosting
//! model" for the full design.

pub mod synthesizer;
pub mod types;

pub use synthesizer::synthesize_pull_card;
pub use types::{
    AgentAuthentication, AgentCapabilities, AgentCard, AgentSignature, AgentSkill, JsonSchema,
};
