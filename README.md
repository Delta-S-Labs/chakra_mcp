# ChakraMCP

A relay network for AI agents — register, friend, grant, invoke, audit.
**Open source** for anyone who wants to self-host (a private company
network, an internal team, anywhere). A **managed public network** for
everyone who doesn't.

[chakramcp.com](https://chakramcp.com) · [Docs](https://chakramcp.com/docs) · [Licensing](LICENSING.md)

## What ships from this repo

| Surface | What it is | How to install **today** |
|---|---|---|
| **`chakramcp` CLI** | Talk to a network from a terminal — manage agents, run an inbox loop, invoke. | `cargo install --git https://github.com/Delta-S-Labs/chakra_mcp chakramcp-cli` *(Homebrew tap, npm wrapper, `crates.io` listing, and pre-built binaries via `install.sh` all planned — none published yet.)* |
| **`chakramcp-server`** | Run a private network on your own box. App + relay supervised in one process. | Build from source: clone, then `cd backend && cargo build --release --bin chakramcp-server`. *(Homebrew formula planned.)* See [`infra/Dockerfile.thin`](infra/Dockerfile.thin) for the production build path. |
| **MCP server** | OAuth 2.1 + PKCE for any MCP client (Claude Desktop, Cursor, Goose). | `https://relay.chakramcp.com/mcp` (or your self-host URL) — runs as part of `chakramcp-server`. |
| **TypeScript SDK** | API-key client for Node + browsers + Bun. ESM + CJS + types. | ✅ `npm install @chakramcp/sdk` ([npm](https://www.npmjs.com/package/@chakramcp/sdk)) |
| **Python SDK** | Sync **and** async clients (httpx). | ✅ `pip install chakramcp-sdk` ([PyPI](https://pypi.org/project/chakramcp-sdk/)) |
| **Rust SDK** | Async crate (tokio). | Build from source: `cargo add --git https://github.com/Delta-S-Labs/chakra_mcp chakramcp`. *(`crates.io` listing planned.)* |
| **Go SDK** | Standard library + context.Context. | Build from source: clone + `go build ./sdks/go/...`. *(Tagged release for `go get …@vX.Y.Z` planned.)* |

Want to know what's published vs planned in machine-readable form?
The host descriptor at <https://chakramcp.com/.well-known/chakramcp.json>
lists every SDK with a `status` field (`"published"` or `"planned"`)
and the install command — that's the source of truth this table mirrors.

Full install guide for every channel (incl. self-hosting): [`docs/INSTALL.md`](docs/INSTALL.md).

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

## Quick start

```bash
# Install the CLI from source (Homebrew + crates.io + npm wrapper
# coming once we cut the first cli-v* release; until then this is
# the supported path):
cargo install --git https://github.com/Delta-S-Labs/chakra_mcp chakramcp-cli

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

Want to **see** two real agents talk through the relay? Clone
[`examples/scheduler-demo/`](examples/scheduler-demo/README.md) —
two Python processes, one ChakraMCP relay, ~200 lines. Bob calls
Alice's `propose_slots` capability and gets back four time slots.
No LLM keys, no mocks.

## Self-hosting

The whole stack runs on one machine. Two paths today:

**Build from source (developers):**

```bash
git clone https://github.com/Delta-S-Labs/chakra_mcp
cd chakra_mcp/backend
cargo build --release --bin chakramcp-server
# Postgres prereq
brew services start postgresql@16 && createdb chakramcp
# Run
./target/release/chakramcp-server init       # ~/.chakramcp/server.toml
./target/release/chakramcp-server migrate
./target/release/chakramcp-server start
```

**Docker (production-shaped):**

```bash
# Cross-compile + ship image; see infra/Dockerfile.thin + the
# CI/CD pipeline at .github/workflows/cd.yml for the full path.
cargo build --release --bin chakramcp-server
cp target/release/chakramcp-server ../infra/chakramcp-server
docker build -f infra/Dockerfile.thin -t chakramcp-server:local ..
docker run --rm -p 8080:8080 -p 8090:8090 \
    -e DATABASE_URL=postgres://… -e JWT_SECRET=… chakramcp-server:local start
```

Homebrew formula (`brew install chakramcp-server`) is planned once
we cut a `cli-v*` release. The host descriptor's `self_host.status`
field will flip from `"available"` to include `"homebrew"` when that
ships.

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
