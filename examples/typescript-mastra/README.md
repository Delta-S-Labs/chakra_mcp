# TypeScript + Mastra example agent

A minimal ChakraMCP agent in TypeScript using
[Mastra](https://mastra.ai/) on top of the [Vercel AI SDK](https://sdk.vercel.ai/).
Talks to NVIDIA NIM via its OpenAI-compatible endpoint.

## Prerequisites

- Node.js 20+ and pnpm 9+
- An `NVIDIA_API_KEY` in the repo's `.env.local`

## Run

```bash
# from the repo root:
task examples:install
task examples:ts

# or directly:
cd examples/typescript-mastra
pnpm install
pnpm agent "What is the relay network in one line?"
```

## What it does

1. Loads `.env.local` from the repo root.
2. Creates a Mastra `Agent` configured with the OpenAI-compatible provider
   pointed at NVIDIA NIM.
3. Sends one prompt, prints the response.
4. **TODO:** registers with the relay, listens for events. Pending Rust
   backend Phase 1.

## Why Mastra

Mastra is a TypeScript-first agent framework with first-class support for
tools, workflows, memory, and evals — built on top of the Vercel AI SDK,
which makes provider switching trivial. It pairs naturally with Next.js
edge runtimes and serverless deployments, which is where most TypeScript
agents live.

## Files

- `package.json` — pnpm-managed dependencies
- `tsconfig.json` — strict mode, ESM
- `src/agent.ts` — the agent
- `src/relay-client.ts` — relay HTTP client stub
