# ChakraMCP — examples

End-to-end agent examples that talk through a real ChakraMCP relay.
Clone, run, watch two processes have a real conversation through the
network.

| Folder | Language | Stack | What it shows |
|---|---|---|---|
| [`scheduler-demo/`](scheduler-demo/) | Python | `chakramcp` SDK (no LLM) | **Start here.** Two real agents through one relay — friendship, grant, `inbox.serve()`, `invoke_and_wait()`, trust context. ~200 lines. |
| [`python-langchain/`](python-langchain/) | Python | [LangChain](https://www.langchain.com/) | uv-managed, NVIDIA NIM by default, Bedrock optional. |
| [`rust-rig/`](rust-rig/) | Rust | [rig](https://github.com/0xPlaygrounds/rig) | Cargo workspace member, idiomatic async. |
| [`typescript-mastra/`](typescript-mastra/) | TypeScript | [Mastra](https://mastra.ai/) | pnpm, ESM, Vercel AI SDK under the hood. |

The **scheduler-demo** is the recommended first stop — it exercises the
full relay flow end-to-end with no LLM keys required. The other three
plug an LLM into one of the agents to show real model inference riding
on top of the same wire.

All three agents:

1. Load API keys from `.env.local` at the repo root (`NVIDIA_API_KEY`, `AWS_*`,
   `ANTHROPIC_API_KEY`, etc.)
2. Talk to **NVIDIA NIM** by default — free, no card required, multiple
   strong models. Get a key at [build.nvidia.com](https://build.nvidia.com/).
3. Have a placeholder for relay registration / discovery / messaging.
   The relay backend is Phase 1, not yet scaffolded — the placeholders
   become live calls once it lands.

## Quick start

```bash
# 1. Make sure your .env.local has NVIDIA_API_KEY (or AWS Bedrock creds)
# 2. From the repo root:
task examples:install        # installs all three
task examples:py             # run the Python agent
task examples:rust           # run the Rust agent
task examples:ts             # run the TypeScript agent
```

Each prints an LLM response to a hardcoded prompt and exits. The relay
conversation flow gets wired up once the Rust backend's Phase 1 lands.

## Why three languages?

Because the relay protocol is HTTP and the message envelope is JSON —
nothing about it should be language-locked. Three reference agents
prove it:

- A **Python** agent demonstrates the LangChain ecosystem path. Most
  agent builders today land here.
- A **Rust** agent demonstrates that the same protocol works in the same
  language as the relay itself — useful for service-side agents and for
  contributors who want to add features near the wire.
- A **TypeScript** agent demonstrates the JS ecosystem path. Critical
  for browser-side agents, edge-runtime agents, and Vercel-style
  deployments.

Every new agent feature should land in all three (or have a documented
reason it doesn't).

## Adding a fourth language

If you want to demo Go, Elixir, Ruby — anything — fork one of the
existing examples, port the protocol calls, open a PR. The protocol
spec is in [`docs/chakramcp-build-spec.md`](../docs/chakramcp-build-spec.md).
