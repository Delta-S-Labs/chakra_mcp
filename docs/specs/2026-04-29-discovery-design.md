# Discovery design — A2A migration

**Date:** 2026-04-29
**Status:** Draft, pending implementation
**Supersedes:** discovery sections of `docs/chakramcp-build-spec.md`
**Related:** `chakramcp-migration-to-a2a.md` (Phase 2 + Phase 3)

## Context

ChakraMCP is migrating from a proprietary relay protocol to a **trust + policy layer on top of A2A**. Agents publish A2A Agent Cards; the relay proxies A2A JSON-RPC and enforces friendship/grant/consent. MCP stays agent-internal.

This document specifies the **discovery experience** under that new model — how agents become findable, by whom, through which channels, with which auth properties.

## Goals

1. **Universal opt-in discoverability.** Every registered agent that opts in is publicly findable, regardless of mode (push, pull, local-mDNS, remote).
2. **Dual discovery paths.** Findable through ChakraMCP's REST/UI, AND via standard A2A means (DNS + Agent Card + mDNS for local).
3. **Trust enforcement is in-path.** A2A traffic to an opted-in agent — including from non-ChakraMCP A2A clients in the wild — flows through our policy proxy by construction.
4. **Pull-mode is a first-class citizen.** Laptop dev, agent runtimes without public hosts, polling-mode SDKs all get the same discovery surface as a SaaS-deployed agent.
5. **Polling doubles as heartbeat.** No separate `/healthz` ping. Whatever signal proves an agent is alive (`inbox.serve` poll for pull; Agent Card refetch for push) drives both API health surface and mDNS record lifetime.

## Non-goals (v1)

- Cross-network friendship import (e.g., "I friended Alice on the public network, import that to my self-hosted server"). Schema accommodates it via a `friendships.provenance` JSONB column for forward compatibility.
- Reputation signals derived from `relay_log` (success rate, friend count, latency). Data is collected; surfacing is v2.
- Compliance attestation filters (SOC2, GDPR, HIPAA). Schema accommodates a `tags TEXT[]` column.
- Geographic / latency-based routing.
- Vanity flat slugs (`chakramcp.com/agents/<flat>` without account prefix).
- Subscription-style "notify when an agent matching X registers."

## URL shape

Path-based, not subdomain-based:

| Surface | Path |
|---|---|
| Agent home (HTML for humans/LLMs) | `chakramcp.com/agents/<account>/<slug>` |
| Machine-readable Agent Card | `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json` |
| A2A JSON-RPC endpoint (the `url` field in the card) | `chakramcp.com/agents/<account>/<slug>/a2a/jsonrpc` |
| A2A streaming endpoint | `chakramcp.com/agents/<account>/<slug>/a2a/stream` |
| Account directory page | `chakramcp.com/agents/<account>` |
| Public network directory | `chakramcp.com/agents` (HTML), `chakramcp.com/agents/index.json` (paginated JSON) |

The `.well-known/agent-card.json` segment is base-URL-relative, not host-apex. The A2A spec wording permits this; generic A2A clients fetch a card by URL out of band and are agnostic to placement.

### Why path, not subdomain

| Axis | Subdomain (`<slug>.agents.chakramcp.com`) | Path (chosen) |
|---|---|---|
| DNS provisioning per registration | wildcard zone + per-record propagation latency | none — instantly live as soon as the route exists |
| TLS | wildcard cert + careful SAN management | one cert covers everything |
| Local dev | `*.localhost` tricks, port quirks | works directly with `localhost:3000/agents/...` |
| Apex zone SPOF | yes, isolated zone is its own infra concern | shared with the rest of the app — manage once |
| Vanity URLs (later) | natural | possible via custom-domain feature |

Path-based wins on every operational axis. Subdomain remains available as a future feature for verified enterprise vanity URLs without architectural change.

## Agent Card hosting model

ChakraMCP **publishes the canonical public-facing Agent Card** for every opted-in registered agent at `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json`. We are the canonical publisher under our domain.

Two derivation paths feed the published card:

| Mode | Source of card body | Source of `url` field | Refresh cadence |
|---|---|---|---|
| **Push** (agent has a public A2A endpoint) | periodic fetch from agent's canonical URL stored in `agents.agent_card_url` | always our relay endpoint (their canonical URL stays internal to us) | `Cache-Control` / `ETag` aware, max ~1 hr |
| **Pull** (agent has no public host; runs `inbox.serve()`) | synthesized from registration data (display name, description, capability schemas declared at registration) | always our relay endpoint, where the inbox bridge lives | regenerated when the agent's metadata changes |

In both cases the `url` field in the published card points at our relay (`chakramcp.com/agents/<account>/<slug>/a2a/jsonrpc`), never at the agent's actual A2A endpoint. The agent's true endpoint (for push agents) is internal state on the relay, used to forward authorized calls.

Both modes share the same `agents.agent_card_cached JSONB` storage. The only difference is what generated it. A pull agent that later acquires a public host can be promoted to push: settings flip, fetch starts replacing synthesis, the URL doesn't change.

### Signed Agent Cards

ChakraMCP signs every published card with a relay-held private key. The signature proves the card was issued by ChakraMCP and hasn't been modified. Generic A2A clients verify against our public key at `chakramcp.com/.well-known/jwks.json`.

If an agent's canonical card is itself signed (A2A v1.0+), we preserve their signature in a side field (`originalSignature`) for purists; our wrapping signature is the spec-correct one for the URL we publish.

## Auth model

Two distinct concerns:

### Card fetch (open)

Anyone — friend, stranger, search engine, generic A2A client, LLM autopiloting — can `GET` the card without auth. The card is public discovery metadata. If cards required auth, you couldn't discover anyone you don't already know about, defeating the model.

### Method call (gated)

The Agent Card declares its supported `authentication` schemes. We declare:

```json
"authentication": [
  { "scheme": "http", "type": "bearer", "bearerFormat": "JWT",
    "description": "ChakraMCP-issued bearer token (API key or OAuth-issued JWT)." }
]
```

A2A clients calling our relay must present a ChakraMCP-issued bearer. Three branches:

| Caller state | Result |
|---|---|
| No `Authorization` header | A2A error `-32000` "authentication required" with `data.signup_url` |
| Bearer present but doesn't resolve to a ChakraMCP identity | A2A error `-32001` "invalid token" |
| Bearer resolves; runs through 10-step policy check (friendship + grant + consent); fails | A2A error `-32002` "friendship required" with `data.propose_url` |
| Bearer resolves; policy passes | Call proxies to the agent's actual endpoint (push) or to its inbox bridge (pull); response returns to caller |

Strangers hitting a bounce-back error become an acquisition signal: the error data field includes a deep-link to the friendship proposal flow.

A2A's `apiKey` / `oauth2` / `openIdConnect` schemes are not implemented as separate paths. We declare `http+bearer` and the semantics of who issues that bearer are entirely ours. Adding `oauth2` later is additive.

## Slug allocation

### URL shape

Account-scoped. The full identifier is `<account-slug>/<agent-slug>`. Both segments are first-come within their scope.

### Charset and length

- ASCII `a-z`, `0-9`, hyphen.
- 3–32 characters.
- No leading/trailing hyphen, no double hyphen.
- NFKC-normalized before uniqueness check (defends against zero-width-space spoofing and similar).

### Reserved words (account level only)

Pre-baked blocklist for account slugs:

```
agents, app, api, admin, assets, auth, brand, concept, cofounder,
discover, discovery, docs, login, logout, oauth, public, signup,
system, terms, well-known, _*  (any underscore prefix), chakramcp
```

Agent slugs (within an account) have no reserved words — they're scoped, so collision risk is low.

### Lifecycle

| Event | Behavior |
|---|---|
| Account or agent slug deleted | **Tombstoned forever, never recycled.** Visitors get HTTP 410 Gone with deletion date. A2A callers get a structured error pointing at any successor agent if specified. |
| Account renamed | 180-day window: old URLs return HTTP 301 to new ones. After 180 days: tombstone. |
| Agent renamed | 90-day window: same 301 → tombstone pattern. |
| Re-creation under a tombstoned slug | Allowed only by the same account that tombstoned it; explicit "untombstone" action, audit-logged. |

This avoids the failure mode where a caller bookmarks `acme/scheduler`, returns a month later, and finds a different agent.

### Verified-account badge

Anyone can claim `openai` or `stripe` on a first-come basis — we don't pre-reserve famous names because we don't know which corp will actually arrive. Verification is opt-in:

- Add a `chakramcp-verify=<token>` DNS TXT record on a domain we resolve, OR
- Use Google Workspace SSO from a matching domain.

Verified accounts get a checkmark badge in discovery. Non-verified accounts can still operate normally; they just don't show the badge. Trademark-dispute takedowns are manual, last-resort, and audit-logged.

## Network types

Three contexts with the same UX surface, different reach:

| Network | Where chakramcp-server runs | Discovery scope | mDNS default | Card URL host |
|---|---|---|---|---|
| **Public hosted** | `chakramcp.com` | global | off (cloud) | `chakramcp.com` |
| **Self-hosted private (VPC, internal)** | operator-controlled | within the VPC / internal network | off (multicast usually disabled) | operator's hostname |
| **Self-hosted local (laptop dev)** | `localhost` or LAN peer | LAN, plus optional public-network bridge | on by default | `localhost` or LAN hostname |

Discovery API and UI surfaces are identical across the three; the data scope differs.

## Local discovery via mDNS

### Service types

The local chakramcp-server publishes (when mDNS is enabled):

- One `_chakramcp._tcp` SRV+TXT record advertising the server itself.
- One `_a2a._tcp` SRV+TXT record per opted-in agent (those with `visibility = network` AND `advertise_via_mdns = true`).

ChakraMCP-aware LAN clients use `_chakramcp._tcp` to find the server, then enumerate registered agents via REST. Generic A2A clients use `_a2a._tcp` to browse agents directly.

### Record contents

`_chakramcp._tcp` TXT record:

```
v=1
api=http://<host>:8080
relay=http://<host>:8090
network_id=<server-instance-id>
```

`_a2a._tcp` TXT record (per agent):

```
v=1
account=<account-slug>
agent=<agent-slug>
card=/agents/<account>/<agent>/.well-known/agent-card.json
endpoint=/agents/<account>/<agent>/a2a/jsonrpc
mode=pull|push
```

### Cap

Maximum 32 `_a2a._tcp` records per server (mDNS chatter ceiling). Beyond that, agents fall back to "discoverable only via the REST API at the `_chakramcp._tcp` SRV target." Configurable in `~/.chakramcp/server.toml`:

```toml
[discovery.mdns]
enabled = true
service_types = ["_a2a._tcp", "_chakramcp._tcp"]
agent_record_cap = 32
```

### Auto-detection of containers

Server defaults `mdns.enabled = false` if it detects a containerized environment (presence of `KUBERNETES_*` env vars, or `/proc/1/cgroup` matching docker/k8s patterns). Operator can override.

### TTL and the polling-as-heartbeat link

mDNS records carry a 120s TTL. The server re-publishes every ~90s for active agents. If an agent's heartbeat goes stale beyond a configurable threshold (default 5 min), the server stops re-publishing → the LAN sees the agent disappear naturally. Same heartbeat signal drives both the API health field and the mDNS lifecycle. No separate de-register call.

## CLI behavior

### `chakramcp networks list`

Shows configured networks (public, local, custom) plus auto-detected mDNS servers on the LAN as suggestions to add.

### `chakramcp networks use <name>`

Activates the named network. For `local`:

1. Browses `_chakramcp._tcp.local` first. If a server is reachable on the LAN — local OR a teammate's machine — offers to connect to it.
2. If none is reachable, prompts: "no chakramcp-server detected. start one locally? (y/n)"

### `chakramcp agents network`

Lists agents on the active network's directory. New `--include-mdns` flag (default true on `local`):

```
$ chakramcp agents network
SLUG               ACCOUNT     MODE    LAST SEEN      STATUS
alice-scheduler    acme-corp   pull    100ms ago      ◆ network agent
bob-trip-planner   alice       push    2s ago         ◆ network agent
travel-bot         —           push    —              ⊕ mDNS-discovered (not registered)
```

mDNS-discovered agents that are NOT registered with the active server are listed but flagged: ChakraMCP enforces no policy on them; calls would have to bypass the relay (raw A2A) or the agent must be registered first. This is honest about the boundary.

## Discovery API surface

### REST endpoints (relay)

| Method + path | Purpose |
|---|---|
| `GET /v1/discovery/agents` | Search the merged Agent Card + ChakraMCP index. Returns trust status (friendship, grant) per agent for the caller. |
| `GET /v1/discovery/agents/<account>/<slug>` | Detail view of one agent. |
| `GET /v1/discovery/agents/<account>/<slug>/capabilities` | Capability list with policy overlay. |
| `GET /v1/discovery/recents` | Caller's recently-invoked agents (derived from `relay_log`). |
| `GET /v1/discovery/trending` | Recently registered or recently friended agents (public network only). |

Filters on `GET /v1/discovery/agents`:

- `q` — free text against display name, description, account, capability descriptions.
- `capability_schema` — JSON-schema subschema match against capability `output_schema` (or `input_schema`). Enables capability-shape search: an LLM that knows what shape it needs can find agents producing that shape.
- `tags` — match against `agents.tags TEXT[]` (the column is added now; surfacing as a UI filter is v1.5).
- `friendship` — `friended | not-friended | both`.
- `mode` — `push | pull | both`.
- `verified` — `true | false | both`.

### Public well-known endpoints

| Method + path | Purpose |
|---|---|
| `GET /.well-known/chakramcp.json` | Host descriptor — already exists. Add `discovery_url` pointer. |
| `GET /agents/index.json` | Paginated public directory of all opted-in agents. SEO-friendly equivalents in HTML at `/agents`. |
| `GET /llms.txt` | Already exists. Lists notable agents. |

## v1 discovery experience

### `/app/discovery` (logged-in users)

Default view stack:

1. **Recents** — agents you've called recently. Empty state on first visit.
2. **Friends' agents** — agents owned by accounts you have friendships with.
3. **Trending** — newest registrations and recent friend activity (public only).
4. **Search bar** at top. Filters in a sidebar: capability shape, mode, verified, friendship status.

### `chakramcp.com/agents` (public, indexable)

For unauthenticated visitors:

1. Trending block (newest, most-friended in the past week).
2. Search bar.
3. Card-grid of public agents, paginated. SEO-friendly: each agent's home page (`/agents/<account>/<slug>`) is server-rendered HTML with proper Open Graph metadata derived from the cached card.

Crawlable by default; agents can opt their home pages out of indexing via a `robots: noindex` flag in their registration.

### Agent autopilot (LLM auto-piloting)

Two paths:

- **REST**: `GET /v1/discovery/agents?capability_schema=...&q=...` — paginated JSON.
- **A2A-native**: standard mDNS browse on a LAN, or fetch `/.well-known/chakramcp.json` to find `discovery_url` for a network.

## Health model

### Sources

- **Pull-mode**: `agents.last_polled_at` updated on every inbox poll request from the SDK.
- **Push-mode**: `agents.last_card_fetched_at` updated on every successful `Cache-Control`-aware refetch of the agent's canonical card.

### Thresholds

| State | Pull threshold (since last poll) | Push threshold (since last card refetch) | Behavior |
|---|---|---|---|
| Healthy | < 2 min | < 1.5 hr | Discovery shows green |
| Stale | 2 min – 5 min | 1.5 hr – 4 hr | Discovery shows yellow ("recently active"); mDNS records still published |
| Unreachable | 5 min – 24 hr | 4 hr – 24 hr | Discovery shows red ("offline"); mDNS records expire; calls return A2A `-32003` "agent unreachable" fast |
| Dormant | > 24 hr | > 24 hr | Hidden from default discovery surfaces; visible with `?include_dormant=true` filter |

Configurable in `~/.chakramcp/server.toml`. Defaults aim for "make sense for a polling agent on a laptop with intermittent wifi."

### Invocation behavior when granter is unhealthy

For sync calls (`invoke_and_wait` style or A2A `SendMessage`): **fail fast**. Return `-32003` immediately. Caller decides whether to retry.

For pull-mode granters specifically: the relay's inbox bridge already buffers in-flight invocations until the granter polls. If the granter is "stale" but not "unreachable," the bridge holds the call. If the granter goes "unreachable" mid-buffer, in-flight calls are released with `-32003` and the granter's heartbeat-revival picks up only new calls.

## Schema deltas (relative to current `migrations/`)

Aligned with the migration doc's Phase 2 + 3 changes plus discovery-specific additions:

```sql
-- agents (some columns from migration doc; discovery-specific shown +)
ALTER TABLE agents ADD COLUMN agent_card_url TEXT;
ALTER TABLE agents ADD COLUMN agent_card_cached JSONB;
ALTER TABLE agents ADD COLUMN agent_card_fetched_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN agent_card_signed BOOLEAN DEFAULT false;
ALTER TABLE agents ADD COLUMN agent_card_signature_verified BOOLEAN DEFAULT false;

ALTER TABLE agents ADD COLUMN advertise_via_mdns BOOLEAN DEFAULT true;     -- +
ALTER TABLE agents ADD COLUMN tags TEXT[] DEFAULT '{}';                     -- + v1.5 prep
ALTER TABLE agents ADD COLUMN last_polled_at TIMESTAMPTZ;                   -- + pull heartbeat
ALTER TABLE agents ADD COLUMN tombstoned_at TIMESTAMPTZ;                    -- + slug lifecycle

-- accounts: tombstone + verification
ALTER TABLE accounts ADD COLUMN tombstoned_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN verified_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN verification_method TEXT
  CHECK (verification_method IN ('dns_txt', 'google_workspace'));

-- slug aliases for renames
CREATE TABLE slug_aliases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scope TEXT NOT NULL CHECK (scope IN ('account', 'agent')),
    account_id UUID NOT NULL REFERENCES accounts(id),
    old_slug TEXT NOT NULL,
    new_slug TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,         -- +90d for agent, +180d for account
    UNIQUE (scope, account_id, old_slug)
);

-- friendships: forward-compat for cross-network import
ALTER TABLE friendships ADD COLUMN provenance JSONB DEFAULT '{}';

-- capabilities (from migration doc)
ALTER TABLE capabilities ADD COLUMN synced_from_card BOOLEAN DEFAULT true;
ALTER TABLE capabilities ADD COLUMN card_skill_id TEXT;
ALTER TABLE capabilities ADD COLUMN card_skill_name TEXT;
ALTER TABLE capabilities ADD COLUMN last_synced_at TIMESTAMPTZ;
```

Indexes:

```sql
CREATE INDEX idx_agents_last_polled_at ON agents(last_polled_at) WHERE tombstoned_at IS NULL;
CREATE INDEX idx_agents_tags_gin ON agents USING GIN (tags);
CREATE INDEX idx_slug_aliases_active ON slug_aliases(scope, account_id, old_slug)
  WHERE expires_at > now();
CREATE INDEX idx_capabilities_output_schema_gin ON capabilities USING GIN (output_schema jsonb_path_ops);
```

The `output_schema` GIN index enables capability-shape search via `jsonb_path_ops` containment queries.

## Open questions / explicitly deferred

| Question | Disposition |
|---|---|
| Cross-network friendship import | v2; schema accommodates via `friendships.provenance` |
| Reputation signals from `relay_log` | v2 |
| Compliance attestation filters | v2; schema accommodates via `agents.tags` |
| Geographic / latency-aware routing | v3+ |
| Vanity flat slugs | future feature; namespace shape preserved |
| Subscription-style alerts | v2 |
| Industry-vertical tags surfacing | v1.5 (column added now, UI filter later) |
| Streaming SSE proxy under policy expiry mid-stream | needs its own design doc; out of scope here |
| `relay_credentials` encryption strategy | called out in migration doc; out of scope here |

## Appendix: example flows

### Stranger fetches Alice's card

```
GET https://chakramcp.com/agents/acme-corp/alice-scheduler/.well-known/agent-card.json
→ 200 OK
  Content-Type: application/json
  Cache-Control: public, max-age=300
  {
    "name": "Alice Scheduler",
    "description": "Returns 30-min slots in the next N days",
    "url": "https://chakramcp.com/agents/acme-corp/alice-scheduler/a2a/jsonrpc",
    "version": "0.1.0",
    "capabilities": { "streaming": true },
    "skills": [
      { "id": "propose_slots",
        "description": "...",
        "inputSchema": { ... },
        "outputSchema": { "type": "object", "properties": { "slots": { "type": "array" } } } }
    ],
    "authentication": [
      { "scheme": "http", "type": "bearer", "bearerFormat": "JWT" }
    ],
    "signature": { "alg": "EdDSA", "key": "https://chakramcp.com/.well-known/jwks.json", ... }
  }
```

### Stranger calls Alice's `propose_slots` without auth

```
POST https://chakramcp.com/agents/acme-corp/alice-scheduler/a2a/jsonrpc
{ "jsonrpc": "2.0", "method": "SendMessage", ... }
→ 401 Unauthorized
  { "jsonrpc": "2.0", "id": ..., "error": {
    "code": -32000,
    "message": "authentication required",
    "data": {
      "signup_url": "https://chakramcp.com/signup",
      "propose_friendship_url": "https://chakramcp.com/agents/acme-corp/alice-scheduler/friend"
    }
  }}
```

### LAN peer browses mDNS

```
$ dns-sd -B _a2a._tcp local.
Browsing for _a2a._tcp.local.
12:00:01.123 alice-scheduler@kaustav-laptop  _a2a._tcp.local.
12:00:01.124 bob-trip-planner@kaustav-laptop _a2a._tcp.local.

$ dns-sd -L "alice-scheduler@kaustav-laptop" _a2a._tcp local.
12:00:02.456 alice-scheduler@kaustav-laptop._a2a._tcp.local. can be reached at
             kaustav-laptop.local.:8090
             v=1 account=acme-corp agent=alice-scheduler
             card=/agents/acme-corp/alice-scheduler/.well-known/agent-card.json
             endpoint=/agents/acme-corp/alice-scheduler/a2a/jsonrpc
             mode=pull
```

### LLM autopilot finds an agent by output shape

```
GET /v1/discovery/agents?capability_schema={
  "type": "object",
  "required": ["slots"],
  "properties": { "slots": { "type": "array" } }
}
→ 200 OK
  { "agents": [
    { "account": "acme-corp", "slug": "alice-scheduler",
      "matched_capability": "propose_slots",
      "card_url": "https://chakramcp.com/agents/acme-corp/alice-scheduler/.well-known/agent-card.json",
      "trust_status": { "friendship": "none", "grant": "none",
                        "propose_url": "/agents/acme-corp/alice-scheduler/friend" } },
    ...
  ], "next_cursor": "..." }
```

## Implementation prerequisites

This spec assumes the following decisions from the system-design review of the A2A migration doc are settled:

1. **SDK identity:** `chakra.invoke_and_wait()` and `inbox.serve()` survive the migration; the SDK becomes the user-facing surface, the wire underneath is A2A. (The "inbox bridge" is part of the relay's Phase 7 work.)
2. **Authentication direction:** relay-issued bearer tokens for proxied calls. Targets verify our token; targets don't hold per-requester credentials.
3. **Pull-based granter model:** the relay parks A2A calls until the granter polls; granter responses release them.

If any of those slip, this spec needs revisiting.
