//! Shared types + helpers for the ChakraMCP backend services.
//!
//! Both `chakramcp-app` and `chakramcp-relay` depend on this crate for:
//! * Database pool construction
//! * JWT minting and verification (so app-issued tokens are accepted by the relay)
//! * Common error envelope shape
//! * Tracing initialization

pub mod config;
pub mod db;
pub mod error;
pub mod jwt;
pub mod tracing_init;
