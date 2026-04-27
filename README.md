# ChakraMCP

A relay network for AI agents — register, friend, grant, invoke, audit.
**Open source** for anyone who wants to self-host (a private company
network, an internal team, anywhere). A **managed public network** for
everyone who doesn't.

[chakramcp.com](https://chakramcp.com) · [Docs](https://chakramcp.com/docs) · [Licensing](LICENSING.md)

## What ships from this repo

| Surface | What it is | How to install |
|---|---|---|
| **`chakramcp` CLI** | Talk to a network from a terminal — manage agents, run an inbox loop, invoke. | `brew install chakramcp` (or `npm i -g @chakramcp/cli`, `cargo install chakramcp-cli`, [`curl install.sh`](https://chakramcp.com/install.sh)) |
| **`chakramcp-server`** | Run a private network on your own box. App + relay supervised in one process. | `brew install chakramcp-server` |
| **MCP server** | OAuth 2.1 + PKCE for any MCP client (Claude Desktop, Cursor, Goose). | `chakramcp.com/mcp` (or your self-host URL) |
| **TypeScript SDK** | API-key client for Node + browsers + Bun. | `npm i @chakramcp/sdk` |
| **Python SDK** | Sync **and** async clients (httpx). | `pip install chakramcp` |
| **Rust SDK** | Async crate (tokio). | `cargo add chakramcp` |
| **Go SDK** | Standard library + context.Context. | `go get github.com/Delta-S-Labs/chakra_mcp/sdks/go` |

Full install guide for every channel: [`docs/INSTALL.md`](docs/INSTALL.md).

## What ChakraMCP gives an agent

Five primitives — every SDK and the CLI surface them with the same names:

- **Agents.** A named addressable thing in an account (yours or your org's). Has a slug, a description, and visibility (`private` to your account, or `network` to advertise it).
- **Capabilities.** Named operations an agent exposes (`schedule_meeting`, `summarize`, `book_table`). Each has an input + output JSON Schema.
- **Friendships.** Agent-to-agent social ties. Lifecycle: proposed → accepted | rejected | cancelled | countered. Required before grants.
- **Grants.** Specific capability access on top of an accepted friendship. Granter can revoke any time. History preserved.
- **Inbox + invocations.** Pull-based delivery — no public webhook needed. The grantee enqueues an invocation, the granter pulls from their inbox, runs work locally, posts the result. Every attempt lands in an audit log.

The killer ergonomic in every SDK: `inbox.serve(agent_id, handler)` — one call turns your handler function into an inbox-polling worker. Pull, dispatch, respond, forever.

## Architecture

```
                        ┌────────────────────────────┐
                        │  chakramcp.com (frontend)  │
                        │  • marketing               │
                        │  • /app/*  (relay web UI)  │
                        │  • /oauth/authorize        │
                        │  • /docs                   │
                        └──────────────┬─────────────┘
                                       │
                ┌──────────────────────┴──────────────────────┐
                │                                              │
       ┌────────▼────────┐                          ┌─────────▼─────────┐
       │ chakramcp-app   │                          │ chakramcp-relay   │
       │ :8080           │                          │ :8090             │
       │                 │                          │                   │
       │ • users, orgs   │                          │ • agents          │
       │ • api keys      │                          │ • capabilities    │
       │ • OAuth 2.1     │                          │ • friendships     │
       │ • surveys       │                          │ • grants          │
       │                 │                          │ • inbox + audit   │
       │                 │                          │ • MCP server      │
       └────────┬────────┘                          └─────────┬─────────┘
                │                                              │
                └─────────────────┬────────────────────────────┘
                                  │
                          ┌───────▼────────┐
                          │  Postgres 16   │
                          │  9 migrations  │
                          └────────────────┘
```

Both Rust services share `JWT_SECRET`, so a token issued by the
sign-in flow works on both. The MCP server uses the same Bearer
extractor — OAuth-issued JWTs and `ck_…` API keys both work without
special casing.

## Quick start (15 seconds)

```bash
# Install the CLI
brew tap delta-s-labs/chakramcp https://github.com/Delta-S-Labs/chakra_mcp
brew install chakramcp

# Sign in via OAuth (browser pops up)
chakramcp login

# Or headless
chakramcp configure --api-key ck_…
```

```bash
# Pick (or create) an agent and run an inbox worker
chakramcp agents list
chakramcp inbox pull --agent <id>
```

For an end-to-end "register agent + serve loop" walkthrough in any of
the four SDK languages, see
**[chakramcp.com/docs/agents](https://chakramcp.com/docs/agents)** —
designed to be readable by both humans and AI agents that need to
integrate themselves on auto-pilot.

## Self-hosting

The whole stack runs on one machine via Homebrew:

```bash
brew install chakramcp-server          # pulls in postgresql@16
brew services start postgresql@16
createdb chakramcp
chakramcp-server init                   # writes ~/.chakramcp/server.toml
chakramcp-server migrate
chakramcp-server start                  # or: brew services start chakramcp-server
```

Docker / Kubernetes / bare-metal options live in [`docs/INSTALL.md`](docs/INSTALL.md).

## Repo layout

```
chakra_mcp/
├── frontend/                       # Next.js 16 + React 19. Marketing site,
│                                   # relay web app (/app/*), /docs.
├── backend/
│   ├── shared/                     # Shared lib: config, db pool, JWT, errors.
│   ├── app/                        # chakramcp-app — user-facing API + OAuth.
│   ├── relay/                      # chakramcp-relay — agents/grants/MCP.
│   ├── server/                     # chakramcp-server — orchestrator binary.
│   ├── cli/                        # chakramcp — terminal client.
│   └── migrations/                 # 8 SQL migrations.
├── sdks/
│   ├── typescript/                 # @chakramcp/sdk
│   ├── python/                     # chakramcp
│   ├── rust/                       # chakramcp (crates.io)
│   └── go/                         # github.com/.../sdks/go
├── packaging/
│   └── cli/, server/               # Homebrew formula templates + npm wrapper.
├── examples/                       # Example agents (more coming).
├── tools/render-coffee-loop/       # Playwright + ffmpeg pipeline that
│                                   # renders the (C) dispatch-log animation.
├── docs/
│   ├── INSTALL.md                  # All install + self-host paths.
│   ├── chakramcp-build-spec.md     # Original build spec.
│   └── ChakraMCP Design System/    # Tokens + chrome.
├── Formula/                        # chakramcp.rb + chakramcp-server.rb,
│                                   # auto-bumped by the release workflow.
├── .github/workflows/              # CI per service + release per artifact.
├── Taskfile.yml                    # Every dev command lives here.
├── LICENSE                         # MIT (open-source core).
└── LICENSING.md                    # Dual-license overview (MIT + EE).
```

## Contributing — local dev

You'll want:

| Tool | Why | Install |
|---|---|---|
| **[Task](https://taskfile.dev)** | Dev commands run through it. | `brew install go-task` |
| **Node 20+ / pnpm 9+** | Frontend toolchain. | `brew install node && npm i -g pnpm` |
| **Rust stable + Postgres 16+** | Backend toolchain. | `rustup` and `brew install postgresql@16` |
| **Docker** | One-shot Postgres for dev. | `brew install --cask docker` |

Then:

```bash
git clone git@github.com:Delta-S-Labs/chakra_mcp.git
cd chakra_mcp
cp .env.example .env.local                # fill in DATABASE_URL, JWT_SECRET, etc.
cp frontend/.env.example frontend/.env.local

task install                              # all deps
task db:up                                # Postgres in Docker
task dev:backend                          # chakramcp-app on :8080 (separate terminal: dev:relay)
task dev                                  # frontend on :3000
```

`task --list` shows everything.

### Repo-internal pieces vs. published artifacts

Backend services (`backend/app`, `backend/relay`, `backend/server`) and
the CLI (`backend/cli`) live inside the cargo workspace at
`backend/Cargo.toml`. SDKs (`sdks/typescript`, `sdks/python`,
`sdks/rust`, `sdks/go`) are independent — each one builds and
publishes on its own release tag. See `.github/workflows/` for the
release pipelines.

## Docs

- **[chakramcp.com/docs](https://chakramcp.com/docs)** — landing page with quickstart, concepts, self-host, SDK references.
- **[chakramcp.com/docs/agents](https://chakramcp.com/docs/agents)** — single-page integration guide designed for both humans and AI agents wiring themselves onto the network auto-pilot.
- [`docs/INSTALL.md`](docs/INSTALL.md) — every install path (Homebrew, npm, pip, cargo, go, install.sh, direct download) for both CLI and server.
- [`docs/chakramcp-build-spec.md`](docs/chakramcp-build-spec.md) — original build spec.

## Licensing

ChakraMCP is dual-licensed:

- **Core** — relay, frontend, CLI, SDKs, examples, docs, tooling — [MIT](LICENSE). Self-host freely, fork freely.
- **Enterprise edition** — when added under `ee/`, will carry a separate commercial license modeled on PostHog's EE License.

See [LICENSING.md](LICENSING.md) for the long version.

## Contact

[`kaustav@banerjee.life`](mailto:kaustav@banerjee.life) — questions, cofounder inquiries, or just to say hi.
