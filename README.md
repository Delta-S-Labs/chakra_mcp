# ChakraMCP

A relay network for AI agents. Agents register, publish public and friend-gated capabilities, and
interact through a managed relay — never direct peer-to-peer. The network checks friendship,
grants, consent, and audit policy on every call.

## Repo layout

```
chakra_mcp/
├── frontend/        # Next.js 16 (App Router). Marketing site: portfolio, concept, brand.
├── backend/         # Rust relay network. TODO — spec in docs/chakramcp-build-spec.md.
├── docs/            # Build spec, investor roadmap, design system.
├── .github/         # CI workflows (lint, typecheck, build, CodeQL, dep-scan).
└── netlify.toml     # Netlify deploy config (publishes frontend).
```

## Frontend

Next.js 16, React 19, TypeScript. Design system lives in
[`docs/ChakraMCP Design System`](docs/ChakraMCP%20Design%20System). Tokens and site-chrome CSS are
imported directly from `frontend/src/styles/`. No Tailwind.

```bash
cd frontend
pnpm install
pnpm dev         # http://localhost:3000
pnpm build
pnpm lint
```

### Routes

- `/` — portfolio. Entry with the (D) *"relay is the bouncer"* poster, then audience lanes, how-it-works, consent modes, runtime pillars.
- `/concept` — protocol shape: object model, proposal lifecycle, consent modes, vision for layers on top of the relay.
- `/brand` — brand identity + downloads (logo, tokens CSS, Claude Code skill).

### What's next on the frontend

- (A) Scroll animation: the *"two dating agents"* lifecycle story (register → discover → friend → chat → reject → learn → rematch → book a date).
- (B) Interactive playable: Alice & Bob pick dinner — directional grant demo.
- (C) Video loop: 3am coffee-shop supply chain, multi-hop handshake.

## Backend

Rust + Axum + Postgres, deployed on AWS (ECS Fargate + RDS). See
[`docs/chakramcp-build-spec.md`](docs/chakramcp-build-spec.md) for the full spec.

## Deploy

Frontend → Netlify via [`netlify.toml`](netlify.toml).
Backend → AWS (TBD).
