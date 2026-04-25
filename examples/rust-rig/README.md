# Rust + rig example agent

A minimal ChakraMCP agent in Rust using [rig](https://github.com/0xPlaygrounds/rig).
Talks to NVIDIA NIM via its OpenAI-compatible endpoint.

## Prerequisites

- Rust + Cargo — `curl https://sh.rustup.rs -sSf | sh`
- An `NVIDIA_API_KEY` in the repo's `.env.local`

## Run

```bash
# from the repo root:
task examples:install
task examples:rust

# or directly:
cd examples/rust-rig
cargo run -- "What is the relay network in one line?"
```

## What it does

1. Loads `.env.local` from the repo root.
2. Builds a rig agent pointed at NVIDIA's OpenAI-compatible endpoint.
3. Sends one prompt, prints the response.
4. **TODO:** registers with the relay, listens for events. Pending Rust
   backend Phase 1.

## Why rig

[rig](https://github.com/0xPlaygrounds/rig) is the most actively maintained
Rust agent framework — supports OpenAI / Anthropic / Cohere / OpenAI-compatible
endpoints (which is how we hit NVIDIA), with idiomatic async via Tokio. It's
in the same Tokio + async-trait + serde world as the eventual relay
backend, so the dev environment stays consistent.

## Files

- `Cargo.toml` — dependencies (rig-core, tokio, dotenvy, anyhow)
- `src/main.rs` — the agent
- `src/relay.rs` — relay HTTP client stub
