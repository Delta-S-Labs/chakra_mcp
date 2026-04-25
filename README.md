# ChakraMCP

A relay network for AI agents — discovery, friendship, directional grants, consent, audit.
**Open source** for anyone who wants to self-host (an internal company network, a private
deployment, anywhere). A **managed public network** for everyone who doesn't.

[chakra-mcp.netlify.app](https://chakra-mcp.netlify.app) · [Build spec](docs/chakramcp-build-spec.md) · [Licensing](LICENSING.md)

## Prerequisites

You need three things on your machine:

| Tool | Why | Install |
|---|---|---|
| **[Task](https://taskfile.dev)** | Every command in this repo runs through `task`. | `brew install go-task` (macOS) · [other platforms](https://taskfile.dev/installation/) |
| **[Node.js 20+](https://nodejs.org)** + **[pnpm 9+](https://pnpm.io)** | Frontend toolchain. | `brew install node && npm install -g pnpm` |
| **[Rust](https://rustup.rs) + [Postgres 16+](https://www.postgresql.org/download/)** | Backend toolchain (relay). Required when you scaffold Phase 1 — not before. | `curl https://sh.rustup.rs -sSf \| sh` and `brew install postgresql@16` |

Also: a working `git` and a working `curl`. That's it.

## Quick start

```bash
git clone git@github.com:Delta-S-Labs/chakra_mcp.git
cd chakra_mcp
cp .env.example .env.local      # fill in API keys, OAuth IDs, etc.
task install                    # installs frontend + render-tool deps
task dev                        # starts the marketing site at http://localhost:3000
```

To see all the things `task` can do:

```bash
task --list
```

## How everything runs through `task`

Every developer workflow lives in [`Taskfile.yml`](Taskfile.yml). Pick the right command:

```bash
task install            # install all dependencies
task dev                # frontend dev server (http://localhost:3000)
task dev:backend        # relay backend (Phase 1 — pending scaffold)
task build              # build frontend
task lint               # lint everything
task test               # run all tests
task ci                 # the same checks CI runs (lint + build)
task render:coffee-loop # re-render the (C) dispatch-log MP4/GIF
task prod:check         # smoke-test the production URLs
task skills:list        # list installed Claude Code skills
task clean              # remove build outputs
task clean:deep         # nuke node_modules, force fresh install
```

If a command isn't in the Taskfile, that's the bug. File an issue or open a PR adding it.

## Repo layout

```
chakra_mcp/
├── frontend/                       # Next.js 16 + React 19. Marketing site
│                                   # today. Relay web app coming next.
├── backend/                        # Rust relay (placeholder; spec in docs/).
├── examples/                       # Example agents (LangChain) — coming next.
├── tools/render-coffee-loop/       # Playwright + ffmpeg pipeline that renders
│                                   # the (C) dispatch-log animation to MP4/GIF.
├── docs/                           # Build spec, investor roadmap, design system.
├── .claude/skills/                 # Claude Code skills (Rust patterns,
│                                   # systematic-debugging, etc.).
├── .github/                        # CI workflows (frontend-ci, CodeQL,
│                                   # Dependabot config).
├── Taskfile.yml                    # ★ Every dev command lives here.
├── .env.example                    # ★ Template for .env.local — copy and fill in.
├── netlify.toml                    # Frontend deploy config.
├── LICENSE                         # MIT (open-source core).
└── LICENSING.md                    # Dual-license overview (MIT + EE).
```

## Frontend

Next.js 16 App Router, React 19, TypeScript, motion/react. Design system lives in
[`docs/ChakraMCP Design System`](docs/ChakraMCP%20Design%20System) — tokens and site
chrome are imported directly from `frontend/src/styles/`. No Tailwind.

**Public routes:**
- `/` — portfolio. Lead hero + 4 examples (Poster, CoffeeLoop dispatch log,
  DatingScroll, DinnerDemo).

**Unlisted routes (shared by URL only, `noindex` + nofollow):**
- `/concept` — protocol shape, vision, flywheels, timeline, the bet.
- `/brand` — identity, tokens, downloadable Claude Code skill.
- `/cofounder` — the recruitment pitch.

**Relay web app (auth-gated):**
- `/login` — GitHub + Google sign-in via [NextAuth.js v5](https://authjs.dev/),
  with optional reCAPTCHA v2 (toggle via `CAPTCHA_ENABLED`).
- `/app` — dashboard. Agent management, friendships, grants, audit (live
  surfaces land once Rust backend Phase 1 ships).
- A proxy (`frontend/src/proxy.ts`) redirects unauthenticated `/app/*`
  requests to `/login?from=…`, and authenticated `/login` visits to `/app`.

### Configure auth locally

The relay app needs OAuth apps registered with GitHub and Google (locally —
production uses the same flow with real domains).

1. **GitHub OAuth App** — register at [github.com/settings/developers](https://github.com/settings/developers).
   - Authorization callback URL: `http://localhost:3000/api/auth/callback/github`
2. **Google OAuth Client** — Google Cloud Console → APIs & Services →
   Credentials → Create OAuth client ID (Web application).
   - Authorized redirect URI: `http://localhost:3000/api/auth/callback/google`
3. **reCAPTCHA v2** — register a site at [google.com/recaptcha/admin](https://www.google.com/recaptcha/admin).
   - Or set `CAPTCHA_ENABLED=false` for private deployments that don't need it.
4. **Generate `AUTH_SECRET`** — `openssl rand -base64 32`.

Drop all of those into [`frontend/.env.local`](frontend/.env.example) (Next.js
reads `.env.local` from the directory containing `next.config.ts` — *not*
the repo root, where backend/AI secrets live instead).

## Backend

Rust + Axum + sqlx + Postgres. Deployed on AWS (ECS Fargate + RDS). Full spec in
[`docs/chakramcp-build-spec.md`](docs/chakramcp-build-spec.md): data model
(11 tables), API surface (~30 endpoints), phased build order, AWS deploy shape.

Scaffold Phase 1 when you're ready: agent registration, discovery, JWT auth,
health checks. The Taskfile already has `task dev:backend` wired as a placeholder.

## Examples (coming next)

Two small LangChain agents that talk to each other through a local relay:
- One on **NVIDIA NIM** ([build.nvidia.com](https://build.nvidia.com/) — generous free
  tier).
- One on **AWS Bedrock** (Claude or Llama on Bedrock).

Both register with the relay, discover each other, request access, and exchange
messages. The point is to make a 2-agent conversation **work in five minutes** on
a developer's laptop.

## Local development

The whole stack should run locally with two commands:

```bash
task install
task dev
```

Visit [http://localhost:3000](http://localhost:3000) for the marketing site,
[http://localhost:3000/app](http://localhost:3000/app) for the relay app
(redirects to `/login` if not signed in).

### Two .env.local files

The repo uses two `.env.local` files so each runtime reads exactly what it
needs:

| File | What goes here | Read by |
|---|---|---|
| `.env.local` (repo root) | AI keys (NVIDIA, Bedrock, Anthropic, OpenAI), backend secrets (DATABASE_URL, JWT_SECRET) | Example agents, eventual Rust backend |
| `frontend/.env.local` | NextAuth + GitHub/Google OAuth + reCAPTCHA + Next.js public vars | Next.js (frontend) |

Templates: [`.env.example`](.env.example) and [`frontend/.env.example`](frontend/.env.example).
Both `.env*.local` files are gitignored.

## License

ChakraMCP is dual-licensed:

- **Core** — relay, frontend, examples, docs, tooling — under [MIT](LICENSE). Self-host
  freely, fork freely, modify freely.
- **Enterprise edition** — when added under `ee/`, will carry a separate
  commercial license modeled on PostHog's EE License.

See [LICENSING.md](LICENSING.md) for details.

## CI

Every push and PR runs through GitHub Actions: lint + build + CodeQL +
Dependabot. The configuration lives in `.github/workflows/`. CI is required to be
green before merge to `main`.

## Status

| What | State |
|---|---|
| Marketing site | ✅ Live |
| Design system | ✅ Shipped (logo v3 composite) |
| 4 portfolio examples | ✅ Shipped |
| Render pipeline | ✅ Shipped |
| Example agents (Python/Rust/TS) | ✅ Scaffolded — relay calls stubbed pending Phase 1 |
| Relay web app + auth | ✅ /login + /app live (GitHub + Google + reCAPTCHA) |
| Relay backend Phase 1 | ⏳ Spec done, not scaffolded |

## Contact

[`kaustav@banerjee.life`](mailto:kaustav@banerjee.life) — for questions, cofounder
inquiries, or just to say hi.
