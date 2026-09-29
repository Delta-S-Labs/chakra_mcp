//! Shared types + helpers for the ChakraMCP backend services.
//!
//! Both `chakramcp-app` and `chakramcp-relay` depend on this crate for:
//! * Database pool construction
//! * JWT minting and verification (so app-issued tokens are accepted by the relay)
//! * Common error envelope shape
//! * Scoped agent-grant resolution (the shared scope guard, so REST and
//!   MCP enforce `agent_scope` identically)
//! * Telemetry: tracing init (text/JSON logs), Prometheus metrics, request IDs

pub mod auto_friendship;
pub mod config;
pub mod db;
pub mod error;
pub mod jwt;
pub mod scope;
pub mod telemetry;
