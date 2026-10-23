//! Shared types + helpers for the ChakraMCP backend services.
//!
//! Both `chakramcp-app` and `chakramcp-relay` depend on this crate for:
//! * Database pool construction
//! * JWT minting and verification (so app-issued tokens are accepted by the relay)
//! * Common error envelope shape
//! * Scoped agent-grant resolution (the shared scope guard, so REST and
//!   MCP enforce `agent_scope` identically)
//! * Telling a person's web session from delegated tokens (`sessions`)
//! * Owner consent decisions (`consent`), for the relay's API and the app's
//!   built-in approval page
//! * Message safety's code pass (`safety`): walking, decoding, contact and
//!   opaque-content detection, language, the Jev budget and redaction
//! * The System One (TypeSafe Jev) client and its questions (`system_one`),
//!   for the relay's compliance checks and safety screening
//! * Telemetry: tracing init (text/JSON logs), Prometheus metrics, request IDs

pub mod auto_friendship;
pub mod config;
pub mod consent;
pub mod credits;
pub mod db;
pub mod error;
pub mod hosting;
pub mod jwt;
pub mod safety;
pub mod scope;
pub mod sessions;
pub mod system_one;
pub mod telemetry;
