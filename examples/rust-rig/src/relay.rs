//! ChakraMCP relay client (Rust).
//!
//! Stub for Phase 1. Once the Rust relay ships, this becomes a thin
//! reqwest wrapper around the relay's HTTP API. For now, every method
//! returns Err with a clear message.

use anyhow::{anyhow, Result};

#[allow(dead_code)]
pub struct RelayClient {
    pub base_url: String,
    pub api_token: Option<String>,
}

#[allow(dead_code)]
impl RelayClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_token: None,
        }
    }

    /// Used by `main` to keep the import warning at bay until Phase 1 lands.
    pub fn placeholder() -> &'static str {
        "relay-stub"
    }

    pub async fn register_agent(&self, _name: &str, _capabilities: &[&str]) -> Result<()> {
        Err(anyhow!(
            "Pending Rust relay Phase 1 — see docs/chakramcp-build-spec.md"
        ))
    }

    pub async fn discover(&self, _query: &str) -> Result<Vec<String>> {
        Err(anyhow!("Pending Rust relay Phase 1"))
    }

    pub async fn request_access(&self, _target: &str, _capability: &str) -> Result<()> {
        Err(anyhow!("Pending Rust relay Phase 1"))
    }

    pub async fn call_capability(
        &self,
        _target: &str,
        _capability: &str,
        _payload: serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(anyhow!("Pending Rust relay Phase 1"))
    }

    pub async fn poll_events(&self) -> Result<Vec<serde_json::Value>> {
        Err(anyhow!("Pending Rust relay Phase 1"))
    }
}
