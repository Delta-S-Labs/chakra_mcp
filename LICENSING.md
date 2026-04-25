# Licensing

ChakraMCP follows the same dual-license pattern as
[PostHog](https://github.com/posthog/posthog/blob/main/README.md#license):
the open-source core under a permissive license, with optional enterprise
features under a separate commercial license.

## What's licensed under MIT (the open-source core)

The bulk of this repository is licensed under the [MIT License](LICENSE).
That includes everything currently in:

- `frontend/` — the marketing site and (when built) the relay web app
- `backend/` — the Rust relay network (when scaffolded)
- `tools/` — render pipeline, dev tooling
- `docs/` — design system, build spec, public roadmap
- `.claude/skills/` — installed Claude Code skills (each carries its own
  upstream license; we redistribute under their terms)

You can self-host the relay network anywhere — inside a company, inside a
private network, on your laptop — under MIT. No restrictions, no royalties,
no phoning home. You can also fork it and modify it.

## What's planned for the enterprise license

When we add features specific to multi-tenant, public-network operation —
billing, advanced policy modules, fleet management, analytics, audit
exports — those will live under `ee/` and carry a separate commercial
license modeled on PostHog's Enterprise Edition License (a fair-use license
that prevents repackaging the enterprise features as a competing managed
service).

The `ee/` directory does not yet exist. When it does, this file gets a
specific reference to `ee/LICENSE`.

## What this means in practice

| You want to ... | What you can do |
|---|---|
| Self-host the relay for your own company / network | Yes, freely, under MIT. |
| Fork the repo and modify it | Yes, freely, under MIT. |
| Run a private agent network for your team | Yes, freely, under MIT. |
| Sell ChakraMCP as a competing managed service | Not under the enterprise features (when those exist). The OSS core is yours to fork, but the EE bits won't be. |
| Use the relay code in another open-source project | Yes — keep the MIT notice intact. |

## Public hosted network

The hosted public network at [chakra-mcp.netlify.app](https://chakra-mcp.netlify.app)
(and the eventual production relay) is operated by ChakraMCP / Delta-S-Labs.
That's the managed service. It's not "the license" — it's a separate
commercial offering that uses this OSS code as its foundation.

## Questions

Open a GitHub issue, or email `kaustav@banerjee.life`.
