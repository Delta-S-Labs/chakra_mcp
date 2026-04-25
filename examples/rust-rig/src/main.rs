//! Example ChakraMCP agent — Rust + rig + NVIDIA NIM.
//!
//! Loads NVIDIA_API_KEY from .env.local at the repo root, sends a single
//! prompt to NVIDIA's OpenAI-compatible endpoint via rig, prints the
//! response.
//!
//! The relay registration / discovery / message-send calls live in the
//! `relay` module as stubs. Once the Rust relay's Phase 1 lands (see
//! `docs/chakramcp-build-spec.md`), wire those up.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rig::completion::Prompt;
use rig::providers::openai;

mod relay;

const SYSTEM_PROMPT: &str = "You are a small, well-mannered example agent on the ChakraMCP \
    relay network. Answer in two short sentences.";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Load .env.local from the repo root (three levels up from this file).
    let root_env: PathBuf = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("../../../../.env.local");
    if root_env.exists() {
        let _ = dotenvy::from_path(&root_env);
    }
    let _ = dotenvy::dotenv();

    let nvidia_key = std::env::var("NVIDIA_API_KEY")
        .context("NVIDIA_API_KEY not set. Get a free key at https://build.nvidia.com/ and add it to .env.local")?;
    let base_url = std::env::var("NVIDIA_BASE_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
    let model = std::env::var("NVIDIA_MODEL")
        .unwrap_or_else(|_| "meta/llama-3.1-70b-instruct".to_string());

    // rig's OpenAI provider points at any OpenAI-compatible endpoint.
    let client = openai::Client::from_url(&nvidia_key, &base_url);
    let agent = client
        .agent(&model)
        .preamble(SYSTEM_PROMPT)
        .temperature(0.5)
        .build();

    let prompt = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    let prompt = if prompt.is_empty() {
        "What is the relay network for AI agents in one line?".to_string()
    } else {
        prompt
    };

    let response = agent.prompt(&prompt).await?;
    println!("{response}");

    // TODO: relay integration — pending Rust backend Phase 1.
    // let relay = relay::RelayClient::new(
    //     std::env::var("RELAY_URL").unwrap_or_else(|_| "http://localhost:8080".to_string()),
    // );
    // relay.register_agent("example-rust", &["echo"]).await?;
    // for event in relay.poll_events().await? {
    //     ...
    // }
    let _ = relay::RelayClient::placeholder();

    Ok(())
}
