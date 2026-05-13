# Discovery design — A2A migration

**Date:** 2026-04-29 · **Revision:** 5 · **Status:** Approved by user; D1 + D2a shipped (D2a rev: canonical A2A v0.3 wire format)

> **A2A wire compatibility** — every Agent Card we publish is fully wire-compatible with canonical [A2A v0.3](https://github.com/a2aproject/A2A) (`a2a.proto`). Field names, casing, plurality, required-vs-optional, security-scheme typing, signature shape (JWS / RFC 7515) all match the spec. Generic A2A clients parse our cards without ChakraMCP-specific knowledge. Capability JSON Schemas do NOT live in the card (A2A's `AgentSkill` has no schema fields) — they're served from our REST endpoint at `/v1/discovery/agents/<account>/<slug>/capabilities`. Forward-compat: every type uses `serde(flatten)` to preserve unknown fields across parse + re-serialize, so newer-spec extensions survive republish.
**Supersedes:** discovery sections of `docs/chakramcp-build-spec.md`
**Related:** `chakramcp-migration-to-a2a.md` (Phase 2 + Phase 3 + Phase 7)

---

## Migration-doc overrides (READ FIRST)

The discovery design intentionally amends the upstream migration doc in four places. Where this spec and the migration doc disagree, **this spec wins**, and the migration doc must be updated to match before Phase 7 starts.

| # | Migration doc says | This spec says | Rationale |
|---|---|---|---|
| 1 | `agents.agent_card_url TEXT NOT NULL` | `agents.agent_card_url TEXT` (nullable), with CHECK enforcing card-OR-pull | Pull-mode agents have no canonical card URL by construction. Forcing NOT NULL breaks the pull tier. |
| 2 | Relay endpoint: `POST /relay/{account_id}/{agent_id}` (UUID-based, single A2A surface) | Two distinct surfaces (see "Two-host surface model" below): (a) **A2A surface** at `chakramcp.com/agents/<account-slug>/<agent-slug>/a2a/jsonrpc` — the URL that appears in published cards, hit by external A2A clients. (b) **Legacy ChakraMCP REST surface** at `relay.chakramcp.com/v1/invoke`, `/v1/inbox`, `/v1/invocations/...` — preserved as translating shims so v0.1.0 SDKs keep working. There is **no** UUID-based A2A alias on the relay subdomain; that subdomain serves only the legacy REST shape. Cards and `.well-known/...` are exclusively on `chakramcp.com`. | Two distinct user populations: external A2A clients (need readable, signed, public-canonical URLs) and our own SDK users (need backward-compatible REST). Single host can't serve both cleanly. |
| 3 | Relay holds per-target credentials in `relay_credentials` and forwards calls with the target's auth | Relay holds **its own private signing key**. It mints short-lived bearer JWTs for each forwarded call, signed by the relay. Targets verify our public key via JWKS. The `relay_credentials` table is removed. | Original design lets the requester hold target credentials, defeating mediation. Relay-issued JWTs preserve trust mediation and remove the credential-storage problem. |
| 4 | A2A is the wire for everything | Bearer auth `http+bearer` is the **only** declared scheme on published cards. `apiKey`/`oauth2`/`openIdConnect` are not implemented in v1; can be added later additively. | YAGNI; one auth path is enough for v1. |

Beyond these four, this spec composes additively with the migration doc's Phase 2 / 3 / 7 / 9 schemas and endpoints.

### Migration-doc checklist items NOT to execute

While the upstream migration doc is being amended, implementers must skip these specific checklist items from `chakramcp-migration-to-a2a.md` (they're either wrong or superseded by this spec):

- **Migration doc §1 "Schema change — agents table"** line `agent_card_url TEXT NOT NULL`: skip the `NOT NULL`. This spec's schema is authoritative (nullable).
- **Migration doc §4 "New relay endpoint"** the `relay_credentials` CREATE TABLE block (lines defining encrypted-credential storage): **do not create this table**. Auth direction is reversed (relay-issued JWT to targets, not target-issued cred to relay).
- **Migration doc Migration Checklist > Database** item: "Create `relay_credentials` table" — skip.
- **Migration doc Migration Checklist > New Rust Modules** item: "`src/db/relay_credentials.rs`" — skip.

A follow-up patch to the migration doc (strikethroughs at the items above + cross-reference to this spec) is tracked as a follow-up task; until then, this list is the source of truth for the deltas.

### Two-host surface model

| Host | Serves | Who hits it |
|---|---|---|
| `chakramcp.com` (apex) | Public site, `/agents/<account>/<slug>/...` (cards, A2A endpoints, agent home pages), `/.well-known/jwks.json`, `/.well-known/error-codes.json`, `/.well-known/chakramcp.json` | Web visitors, external A2A clients, our SDK when it constructs URLs from cards |
| `relay.chakramcp.com` | Legacy ChakraMCP REST: `/v1/invoke`, `/v1/inbox`, `/v1/invocations/...`, `/v1/agents/...`, `/v1/grants/...`, etc. | Existing v0.1.0 SDKs, CLI; translates internally to A2A SendMessage etc. before policy + park/forward |

`relay.chakramcp.com` does NOT serve cards or `.well-known/...`. Cards live exclusively at `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json`.

The current SDK default `relay_url = "https://relay.chakramcp.com"` (verified: `sdks/python/src/chakramcp/_async.py` `DEFAULT_RELAY_URL`) is preserved unchanged. Existing SDK calls to `/v1/invoke` keep landing on `relay.chakramcp.com`; the relay translates them to A2A SendMessage internally before the policy/park/forward path. New A2A traffic from outside our SDK ecosystem hits `chakramcp.com/agents/...` directly.

---

## Context

ChakraMCP is pivoting from a proprietary relay protocol to a **trust + policy layer on top of A2A**. Agents speak A2A; the relay enforces friendship/grant/consent. MCP stays agent-internal.

This document specifies the **discovery experience** under that model — how agents become findable, by whom, through which channels, with what auth properties.

## Goals

1. **Universal opt-in discoverability.** Every registered agent that opts in is publicly findable, regardless of mode (push, pull, local-mDNS, remote).
2. **Dual discovery paths.** Findable through ChakraMCP's REST/UI, AND via standard A2A means (URL-based + mDNS for local).
3. **Trust enforcement is in-path.** A2A traffic to an opted-in agent — including from non-ChakraMCP A2A clients in the wild — flows through our policy proxy by construction.
4. **Pull-mode is a first-class citizen.** Laptop dev, agent runtimes without public hosts, polling-mode SDKs all get the same discovery surface as a SaaS-deployed agent.
5. **Polling doubles as heartbeat.** Whatever signal proves an agent is alive (`inbox.serve` poll for pull; Agent Card refetch for push) drives both API health surface and mDNS record lifetime.
6. **Existing v0.1.0 SDKs survive.** `invoke_and_wait()` and `inbox.serve()` keep working; the wire underneath becomes A2A.

## Non-goals (v1)

Cross-network friendship import (schema accommodates via `friendships.provenance`). Reputation signals from `relay_log`. Compliance attestation filters (schema accommodates via `agents.tags`). Geographic / latency-aware routing. Vanity flat slugs. Subscription-style alerts. Streaming SSE proxy under mid-stream policy expiry (separate doc). Full JSON Schema subset matching for capability search (we use JSONB containment in v1; full JSON Schema in v2).

---

## URL shape

Path-based, not subdomain-based.

| Surface | Path |
|---|---|
| Agent home page (HTML) | `chakramcp.com/agents/<account>/<slug>` |
| Agent Card (machine-readable) | `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json` |
| A2A JSON-RPC endpoint (`url` field of card) | `chakramcp.com/agents/<account>/<slug>/a2a/jsonrpc` |
| A2A streaming endpoint | `chakramcp.com/agents/<account>/<slug>/a2a/stream` |
| Account directory page | `chakramcp.com/agents/<account>` |
| Public network directory | `chakramcp.com/agents` (HTML), `chakramcp.com/agents/index.json` (paginated JSON) |
| Legacy SDK REST endpoints (preserved for back-compat) | `relay.chakramcp.com/v1/invoke`, `/v1/inbox`, `/v1/invocations/...` — see "Two-host surface model" |

The legacy endpoints on `relay.chakramcp.com` are NOT A2A — they're the existing ChakraMCP REST shape preserved for the v0.1.0 SDKs. The SDK keeps using them; the relay translates each `/v1/invoke` into an A2A `SendMessage` internally before policy + park/forward. There is no A2A surface (no `a2a/jsonrpc` path, no `.well-known/agent-card.json`) on the `relay` subdomain — A2A traffic from outside our SDK ecosystem goes only to `chakramcp.com/agents/...`.

The `.well-known/agent-card.json` segment is base-URL-relative (per A2A spec wording, "relative to the base URL of the agent"), not host-apex.

### Why path-based, not subdomain

| Axis | Subdomain | Path (chosen) |
|---|---|---|
| DNS provisioning per registration | wildcard zone + per-record propagation | none — instantly live |
| TLS | wildcard cert + careful SAN management | one cert covers everything |
| Local dev | `*.localhost` tricks needed | works directly |
| Vanity URLs (later) | natural | possible via custom-domain feature |

Subdomain remains available for future verified-enterprise vanity URLs without architectural change.

---

## Agent Card hosting model

ChakraMCP **publishes the canonical public-facing Agent Card** for every opted-in registered agent at `chakramcp.com/agents/<account>/<slug>/.well-known/agent-card.json`. We are the canonical publisher under our domain.

Two derivation paths feed the published card:

| Mode | Source of body | Source of `url` field | `synced_from_card` | Refresh |
|---|---|---|---|---|
| **Push** (agent has public A2A endpoint) | periodic fetch from `agents.agent_card_url` | always our relay endpoint | `true` | every 60 minutes (hard cap; upstream `Cache-Control: max-age` directives **clamped to ≤ 60 min** so health thresholds remain meaningful); `ETag` honored within that window; staleness fallback below |
| **Pull** (agent has no public host) | synthesized from registration data + capability rows | always our relay endpoint, where the inbox bridge lives | `false` (came from registration, not a card) | regenerated when registration metadata changes |

In both cases the `url` field points at our relay (`chakramcp.com/agents/<account>/<slug>/a2a/jsonrpc`), never at the agent's actual A2A endpoint. The agent's true endpoint (push only) is internal state, used by the relay to forward authorized calls.

A pull agent that later acquires a public host can be promoted to push without URL change: settings flip, fetch starts replacing synthesis, the URL we publish stays the same.

### Push-mode upstream-card failure

If the agent's canonical card returns 5xx/4xx during refresh:

- First failure: log; serve last-known-good card.
- 5 consecutive failures or `>4 hr since last success`: agent transitions to `Stale` (see Health). Card continues to serve last-known-good.
- `>24 hr since last success`: agent transitions to `Unreachable`. Card serves with a `WARNING` field added to TXT/JSON. Calls return `-32003` fast.
- `>7 days`: agent transitions to `Dormant`, removed from default discovery, card returns 503 with reason. Manual reactivation by operator.

Capability rows from a stale card are NOT removed automatically (avoids data loss on transient outages). Operator can force resync.

### Signed Agent Cards (JWS shape, key rotation)

The relay holds a private Ed25519 signing key (`kid`-rotated). Every published card carries one or more JWS-shaped entries in its `signatures` array (per the A2A v0.3 spec — `AgentCardSignature` is RFC 7515-shaped):

```json
"signatures": [
  {
    "protected": "<base64url-encoded JSON header: { alg: 'EdDSA', kid: 'relay-2026-04' }>",
    "signature": "<base64url-encoded signature bytes>"
  }
]
```

The signature is computed over a canonical-JSON projection of the card body. The projection rules are formalized in the signer module (`backend/relay/src/agent_card/signer.rs`, D2b), but the contract is: **every field that callers need to trust is in scope**, including `supported_interfaces[].url`. Replaying a signed card with a substituted URL fails verification.

JWKS at `chakramcp.com/.well-known/jwks.json` lists active keys with overlap. Rotation cadence: 90 days. Old `kid`s stay in JWKS for 30 days after retirement so cached cards remain verifiable. The `kid` lives in the `protected` header (per RFC 7515, decoded clients pull it out before key lookup); MUST be present on every card.

If an agent's canonical card is itself signed (A2A v1.0+), we preserve the upstream signature objects alongside ours by appending them to the `signatures` array (multiple signatures are explicitly allowed by the spec). Our wrapping signature is the one with our `kid` and binds the card to the URL we publish.

A2A v0.x cards (unsigned) are accepted from upstream agents — we still sign our wrapped output. We never reject an upstream card on signature absence.

### Upstream auth-scheme handling on republish

The relay **always** publishes a single `chakramcp_bearer` entry in `security_schemes` (HTTP+Bearer+JWT) plus a corresponding `security_requirements` reference, regardless of what the upstream canonical card declares. Upstream `api_key` / `oauth2` / `open_id_connect` / `mutual_tls` schemes are not propagated. They have no effect on auth handling because all calls hit our relay first; the relay knows internally how to authenticate forward to the agent's canonical endpoint (the relay-issued JWT it presents to the target — see Override #3).

When a republish happens against a card that declares a non-bearer upstream scheme, the relay emits an admin-side warning event (`chk.publish.unsupported_auth`, see error catalog) so the operator knows the upstream's declared schemes are being ignored. This is informational, not blocking.

---

## Auth model

### Card fetch (open, rate-limited)

Anyone can `GET` the card without auth. Cards are public discovery metadata. Edge cache: `Cache-Control: public, max-age=300`. Rate limit: 60 req/IP/min on the apex domain across all card URLs (defends against fingerprinting via card enumeration).

### Method call (gated)

Cards declare a canonical A2A v0.3 security scheme + requirement (top-level fields, not a flat `authentication` array):

```json
"security_schemes": {
  "chakramcp_bearer": {
    "http": {
      "scheme": "Bearer",
      "bearer_format": "JWT",
      "description": "ChakraMCP-issued bearer token (API key or OAuth-issued JWT)."
    }
  }
},
"security_requirements": [ { "chakramcp_bearer": [] } ]
```

Calls without a bearer or with an invalid bearer fail with structured A2A errors. Calls with a valid bearer go through the **10-step policy decision** (friendship, grant, consent — same algorithm as the migration doc Phase 7).

### A2A error responses

Errors return stable codes plus a `data.code` and a `data.detail` URL. Clients resolve display text via the `data.code` against a published catalog at `chakramcp.com/.well-known/error-codes.json`. Error data **does not** carry user-facing URLs in free-text fields — that surface is phishing-prone (proxies could substitute).

| Code | Meaning | `data.code` | `data.detail` (for clients to follow) |
|---|---|---|---|
| `-32000` | authentication required | `chk.auth.missing` | catalog → signup URL |
| `-32001` | invalid token | `chk.auth.invalid` | (none) |
| `-32002` | friendship required | `chk.policy.friendship_required` | catalog → propose URL pattern |
| `-32003` | grant required | `chk.policy.grant_required` | catalog → grant URL pattern |
| `-32003` | grant revoked mid-flight | `chk.policy.grant_revoked` | (none — granter side only knows; caller learns at task-failure) |
| `-32004` | consent required | `chk.policy.consent_required` | catalog → consent URL pattern |
| `-32005` | agent unreachable | `chk.target.unreachable` | (none) |
| `-32006` | agent tombstoned | `chk.target.tombstoned` | catalog → successor agent if known |
| `-32007` | rate-limited | `chk.rate.limited` | (Retry-After header) |
| `-32008` | wrapped-card auth scheme not bearer | `chk.publish.unsupported_auth` | (admin-side warning when republishing an upstream card that declares a non-bearer scheme; the relay always publishes `http+bearer` regardless) |

`chk.policy.grant_revoked` reuses JSON-RPC code `-32003` because the on-the-wire family is the same ("grant problem"); discriminator is `data.code`. Catalog at `chakramcp.com/.well-known/error-codes.json` is the authoritative list including any future codes added without spec revision.

The data-detail-via-catalog pattern means we can change destination URLs without breaking deployed clients.

### Decision tree (auth + policy)

```
incoming A2A call to /agents/<account>/<slug>/a2a/jsonrpc
└─ no Authorization bearer? → -32000
└─ bearer doesn't resolve to ChakraMCP identity? → -32001
└─ target slug tombstoned? → -32006
└─ target unreachable (see Health)? → -32005 (push) or queue (pull, see Inbox bridge)
└─ caller has friendship with target's account?
   ├─ no → -32002
   └─ yes
      └─ caller's agent has active grant on the target's capability?
         ├─ no → -32003
         └─ yes
            └─ grant requires consent and consent absent/expired? → -32004
            └─ pass → forward to target (push) or park in inbox bridge (pull)
```

This replaces the loose 10-step description in the migration doc. The 10 steps map onto the branches above.

---

## SDK surface preservation

The published v0.1.0 surface (`@chakramcp/sdk`, `chakramcp` Python) keeps working. Mappings:

### `invoke_and_wait()` (sync caller side)

| Surface | Pre-migration | Post-migration |
|---|---|---|
| Caller side | `POST /v1/invoke` then poll `GET /v1/invocations/{id}` | **POST `/agents/<account>/<slug>/a2a/jsonrpc`** carrying A2A `SendMessage`. Relay handles policy + parking + response (see Inbox bridge). The SDK can also continue to use the legacy `/v1/invoke` endpoint as a thin wrapper that translates to A2A internally — the request shape is preserved, the wire underneath changes. |
| Wire | ChakraMCP custom JSON-RPC | A2A JSON-RPC 2.0 |
| Polling | `GET /v1/invocations/{id}` until `status` ∈ {succeeded, failed} | A2A `GetTask` against the same relay endpoint (the SDK does the polling internally; user-visible signature unchanged) |

The SDK's `invoke_and_wait()` signature is unchanged. The SDK accepts both `(grant_id, ...)` and target-by-slug forms.

### `inbox.serve()` (pull granter side)

The Inbox bridge (next section) is the new wire. The SDK polls it and serves A2A calls back through it.

| Surface | Pre-migration | Post-migration |
|---|---|---|
| Granter polls | `GET /v1/inbox?agent_id=<id>` | `GET /v1/inbox?agent_id=<id>` (endpoint preserved; payload now includes the parked A2A `SendMessage` body verbatim under `parked_a2a_request`) |
| Granter responds | `POST /v1/invocations/{id}/result` | `POST /v1/invocations/{id}/result` (endpoint preserved; the relay translates the response back to A2A `Task.completed`/`failed` and releases it to the original A2A caller) |

`inbox.serve()` signature is unchanged. The handler still receives a normalized invocation dict; the new `parked_a2a_request` field gives advanced users access to the raw A2A payload if they need it.

### CLI command preservation

| Command | Behavior change |
|---|---|
| `chakramcp inbox pull --agent <id>` | Unchanged. Polls the same endpoint. |
| `chakramcp invoke --grant <id> ...` | Unchanged from user perspective; under the hood now translates to A2A `SendMessage`. |
| `chakramcp agents create ...` | Updated to accept optional `--agent-card-url` (push) or none (pull). |
| `chakramcp agents network` | Now supports `--include-mdns` (default true on `local`). |

### Schema fact: `relay_log` vs `invocations`

The migration doc renames `capability_runs` to `relay_log`. The discovery spec follows that rename. The `invocations` audit table referenced in the v0.1.0 SDK is the same table viewed through a backward-compatible view; the SDK's `invocations.list/get` endpoints continue to work.

---

## Inbox bridge (pull-mode contract)

The bridge is the load-bearing piece that lets pull-mode agents participate in the A2A wire without a public endpoint.

### Lifecycle of a parked call

1. **Caller** sends A2A `SendMessage` to `chakramcp.com/agents/<account>/<slug>/a2a/jsonrpc`.
2. **Relay** runs the auth + policy decision tree above. On pass, if target is `mode = pull`, the call is **parked**: row inserted in `relay_log` with `policy_decision = 'authorized'`, `parked_at = now()`, `a2a_method = 'SendMessage'`. The relay returns A2A `Task` with `state: working` immediately to the caller.
3. **Granter's SDK** polls `GET /v1/inbox?agent_id=<id>` (preserved endpoint). Response payload preserves the existing `Invocation` dict shape for backward compatibility — new fields are **additive only**. Existing fields (`id`, `capability_name`, `input_preview`, `friendship_context`, `grant_context`, etc.) are unchanged. New additive fields: `parked_a2a_request` (the raw A2A SendMessage body, for advanced users), `parked_at` (timestamp), `parking_deadline_at` (computed from server-wide `inbox_bridge.parking_timeout_s` plus `parked_at`).
4. **Granter's SDK** runs the user's handler, gets a result.
5. **Granter's SDK** posts to `POST /v1/invocations/{id}/result`. The relay translates the result to A2A `Task.completed` or `Task.failed`.
6. **Caller** is observing the task via A2A `GetTask` (the SDK's `invoke_and_wait` does this transparently). On state transition to terminal, caller receives the result.

### Parking timeouts

Single server-wide timeout: `inbox_bridge.parking_timeout_s` in `~/.chakramcp/server.toml`. Default 60 s, bounded `[5, 600]`. Applies to all parked calls regardless of caller, target, or capability.

Rationale for not making this configurable per-grant or per-capability in v1: a 60 s default covers ~all capabilities; truly long-running work should use A2A's native `Task` + `GetTask` pattern (relay returns `state: working` immediately, caller polls), not extended parking. Per-capability or per-grant overrides can be added additively if a real need surfaces.

| Parked duration | Action |
|---|---|
| `< 30 s` | normal |
| `30 s – 5 min` | granter is `Stale` (see Health). Call still parked. |
| `5 min – parking_timeout_s` | call held; bounded by global timeout |
| `≥ parking_timeout_s` | relay releases the call as A2A `Task.failed` with `data.code = chk.target.unreachable`; caller learns; granter's eventual response (if any) is ignored with audit-logged warning |

### State on grant revocation mid-park

If the granter or granter's account revokes the grant or the friendship while a call is parked:

- The parked call is released as `Task.failed` with code `chk.policy.grant_revoked`.
- The granter's next inbox poll does NOT see the call.
- Both events are recorded in `relay_log`.

### State on slug tombstone mid-park

If the target slug is tombstoned while a call is parked:

- The parked call is released as `Task.failed` with code `chk.target.tombstoned`.
- The granter's next inbox poll does NOT see the call.

---

## Slug allocation

### URL shape

`<account-slug>/<agent-slug>`. Both segments first-come within their scope.

### Charset and length

ASCII `a-z`, `0-9`, hyphen. 3–32 characters. No leading/trailing hyphen. No double hyphen. NFKC-normalized before uniqueness check.

### Reserved words (account-slug only)

Pre-baked blocklist:

```
agents, app, api, admin, assets, auth, brand, concept, cofounder,
discover, discovery, docs, login, logout, oauth, public, signup,
system, terms, well-known, _*  (any underscore prefix), chakramcp
```

Agent slugs (within an account) have no reserved words.

### Deletion → tombstone (single decision tree)

```
Tombstone trigger
├─ Voluntary deletion (account owner OR agent owner)
│  └─ Permanent tombstone. Slug NEVER reusable by anyone except the original
│     account, and only via explicit "untombstone" action by the same account
│     within 365 days. After 365 days: permanent.
├─ Account suspended for ToS violation
│  └─ Same as voluntary deletion, plus account-wide tombstone.
└─ Forced takedown (trademark dispute, manual review)
   └─ Slug tombstoned + `forced_takedown_at` timestamp set + slug becomes
      AVAILABLE for re-registration after a 30-day cooling period. The
      complainant does NOT get auto-priority — they must register normally
      after the cooling period (first-come). All takedown decisions
      audit-logged with reason. Manual review only; no v1 self-serve.
```

### Rename redirects

| Scope | Window | After window |
|---|---|---|
| Account rename | 180-day 301 redirect on `/agents/<old>/*` | tombstone |
| Agent rename | 90-day 301 redirect on `/agents/<account>/<old>` | tombstone |

If a rename chain occurs (A→B→C), redirects collapse to head-of-chain (`A` always 301s to `C` while any chain link is alive).

### Verified-account badge

Anyone can claim `openai` or `stripe` first-come. Verification via DNS TXT record (`chakramcp-verify=<token>` on a domain we resolve) OR Google Workspace SSO from a matching domain. Verified accounts get a checkmark; non-verified accounts operate normally.

Trademark-takedown procedure (v1, manual): petition email to `legal@chakramcp.com`. Threshold: registered trademark + clear evidence of impersonation/confusion. SLA: 7 business days. Reviewer: ChakraMCP staff. Decision audit-logged with rationale. The procedure is **explicitly manual in v1** — no self-serve takedown form.

---

## Network types

| Network | Where chakramcp-server runs | Discovery scope | mDNS default | Card URL host |
|---|---|---|---|---|
| **Public hosted** | `chakramcp.com` | global | off (cloud) | `chakramcp.com` |
| **Self-hosted private (VPC)** | operator-controlled | within VPC | off (multicast disabled) | operator's hostname |
| **Self-hosted local (laptop)** | `localhost` or LAN peer | LAN | on by default | `localhost` or LAN hostname |

Discovery API + UI surfaces are identical across networks; only the data scope differs.

---

## Local discovery via mDNS

### Service types

When mDNS is enabled, the server publishes:

- One `_chakramcp._tcp` SRV+TXT record advertising the server itself.
- One `_a2a._tcp` SRV+TXT record per opted-in agent (`visibility = network` AND `advertise_via_mdns = true`), capped at 32. Agents above the cap remain discoverable via REST only; the operator log emits a warning.

### Record contents

`_chakramcp._tcp` TXT:
```
v=1
api=http://<host>:8080
relay=http://<host>:8090
network_id=<persisted UUID v4>
```

`_a2a._tcp` TXT:
```
v=1
account=<account-slug>
agent=<agent-slug>
card=/agents/<account>/<agent>/.well-known/agent-card.json
endpoint=/agents/<account>/<agent>/a2a/jsonrpc
mode=pull|push
```

### `network_id`

A UUID v4 generated on first server boot, persisted to `<state-dir>/network_id`. Stable across restarts unless the state file is deleted (intentional reset).

### Service-name disambiguation on shared LAN

The mDNS service instance name is `<short-host>-<short-network-id-prefix>` (e.g., `kaustav-mbp-a3f9`). Two ChakraMCP servers on the same LAN end up with distinct instance names. CLI dedupes display by `network_id` — different IDs = different servers; same ID = same server (e.g., after a restart with persisted state).

### Cap

Default 32. Configurable in `~/.chakramcp/server.toml`:

```toml
[discovery.mdns]
enabled = true
service_types = ["_a2a._tcp", "_chakramcp._tcp"]
agent_record_cap = 32
```

**Overflow policy** when registered agents exceed the cap: agents are advertised in **registration order** (oldest registered first), with the most recent registrations falling off. This is stable: an agent that's been advertised won't randomly disappear when a new agent is added unless the new agent has an earlier `created_at` (which can't happen normally). The operator log emits a warning once per cap-breach event with the count of dropped agents and a link to docs explaining the cap.

### Container detection

Server defaults `mdns.enabled = false` if it detects a containerized environment (`KUBERNETES_*` env or `/proc/1/cgroup` matching docker/k8s patterns). Operator override available.

### Spoof resistance

Generic A2A clients on the LAN fetch the card from the SRV target, then verify the card's signature against `https://chakramcp.com/.well-known/jwks.json` (or the local server's `_chakramcp._tcp` record's `api=...` host's JWKS endpoint, depending on the network). A spoofer cannot forge our signature without our private key. ChakraMCP-aware clients are advised to verify before trusting any mDNS-discovered card. Documentation in CLI help and SDK docs spells this out.

### Health → mDNS lifecycle

mDNS records have 120s TTL. The server re-publishes every ~90s for active agents. If an agent's heartbeat goes stale (default 5 min), the server stops re-publishing → record expires → LAN sees the agent disappear.

The publisher itself is health-checked: `agents.mdns_advertised_at` is updated on every successful publish. Operator UI surfaces "publisher not advertising in last X minutes" as a warning.

---

## CLI behavior

### `chakramcp networks list`

Configured networks plus auto-detected mDNS servers on the LAN as suggestions to add.

### `chakramcp networks use <name>`

Activates the named network. For `local`:

1. Browses `_chakramcp._tcp.local` first. If a server is reachable on the LAN — local OR a teammate's machine — offers to connect.
2. If none is reachable, prompts to start one locally.

### `chakramcp agents network [--include-mdns]`

Lists agents on the active network's directory. Default `--include-mdns=true` on `local`. mDNS-discovered agents not registered with the active server are listed but flagged:

```
SLUG               ACCOUNT     MODE    LAST SEEN      STATUS
alice-scheduler    acme-corp   pull    100ms ago      ◆ network agent
travel-bot         —           push    —              ⊕ mDNS-discovered (not registered)
```

The `⊕` agents have no trust enforcement. Calls would have to go through raw A2A or the agent must be registered first.

---

## Discovery API surface

### REST endpoints

| Method + path | Purpose | Auth |
|---|---|---|
| `GET /v1/discovery/agents` | Search merged Agent Card + ChakraMCP index. Returns trust status per agent for the caller. | optional bearer (caller's status fields filled in if present) |
| `GET /v1/discovery/agents/<account>/<slug>` | Detail view. | optional bearer |
| `GET /v1/discovery/agents/<account>/<slug>/capabilities` | Capability list with policy overlay. | optional bearer |
| `GET /v1/discovery/recents` | Caller's recently invoked agents. | bearer required |
| `GET /v1/discovery/trending` | Recently registered or recently friended agents. Public network only. | optional bearer |

Filters on `GET /v1/discovery/agents`:

- `q` — free text against display name, description, account, capability descriptions.
- `capability_schema` — **JSONB containment match** against capability `output_schema` or `input_schema`. Uses `@>` operator; full JSON Schema validation is v2.
- `tags` — match against `agents.tags TEXT[]`.
- `friendship` — `friended | not-friended | both` (requires bearer).
- `mode` — `push | pull | both`.
- `verified` — `true | false | both`.
- `include_dormant` — `true` only honored when bearer present (logged-in users only); unauthed defaults to false and ignores the param.

### Public well-known endpoints

| Method + path | Purpose |
|---|---|
| `GET /.well-known/chakramcp.json` | Host descriptor — adds `discovery_url` field. |
| `GET /agents/index.json` | Paginated public directory. Excludes agents with `noindex=true`. |
| `GET /llms.txt` | Already exists. |
| `GET /.well-known/jwks.json` | Relay's signing keys for card-signature verification. |
| `GET /.well-known/error-codes.json` | A2A error code catalog (machine-readable). |

---

## v1 discovery experience

### `/app/discovery` (logged-in)

1. **Recents** — agents you've called recently.
2. **Friends' agents** — agents owned by accounts you have friendships with.
3. **Trending** — newest registrations + recent friend activity (public only).
4. Search bar; filters in sidebar.

### `chakramcp.com/agents` (public, indexable)

1. Trending block.
2. Search bar.
3. Card-grid of public agents, paginated. SEO-friendly: each agent's home page is server-rendered with Open Graph metadata. Crawlable by default; agents with `noindex=true` are excluded.

### Agent autopilot (LLM)

- **REST**: `GET /v1/discovery/agents?capability_schema=...&q=...`
- **A2A-native**: standard mDNS browse on a LAN, or fetch `/.well-known/chakramcp.json` to find `discovery_url`.

---

## Health model

### Sources

- **Pull-mode**: `agents.last_polled_at` updated on every inbox poll. **Server-side `now()`** is the authoritative timestamp; the SDK's clock is ignored to defend against clock skew.
- **Push-mode**: `agents.last_card_fetched_at` updated on every successful refetch.

### State machine (state × mode → behavior)

| Mode | State | Threshold | Discovery surface | Invocation behavior |
|---|---|---|---|---|
| Pull | Healthy | last_polled_at < 2 min ago | green | call parked in inbox bridge |
| Pull | Stale | 2–5 min | yellow ("recently active") | call parked; granter polls within timeout |
| Pull | Unreachable | 5 min – 24 hr | red ("offline") | parked calls released as `-32005` after `grant_timeout_s` |
| Pull | Dormant | > 24 hr | hidden from default; visible with `?include_dormant=true` (logged-in only) | calls fail-fast `-32005` |
| Push | Healthy | last_card_fetched_at < 1.5 hr ago AND no consecutive fetch failures | green | proxied to canonical endpoint |
| Push | Stale | 1.5–4 hr OR 5 consecutive fetch failures | yellow | proxied; capability data may be slightly stale |
| Push | Unreachable | 4–24 hr OR canonical endpoint returning 5xx for >5 min | red | calls fail-fast `-32005` |
| Push | Dormant | > 24 hr | hidden from default | calls fail-fast `-32005` |
| Tombstoned | — | — | hidden | `-32006` |

### Timeline of buffered call when granter degrades

Caller sends call to pull-mode Alice. Alice is `Healthy`. Call parks. Alice's polling stops (laptop sleeps). At `t+2 min` Alice transitions to `Stale` — call still parked, no caller-visible change. At `t+5 min` Alice transitions to `Unreachable`. Calls parked before the transition continue waiting until `grant_timeout_s` (default 60s); the timer started at park time, so most calls already timed out. Calls parked AFTER the transition fail-fast with `-32005`. When Alice resumes polling, calls received in the meantime appear in her next inbox poll only if they're still within `grant_timeout_s`.

---

## Schema deltas

Aligned with migration doc Phase 2/3/7 plus discovery-specific additions. Where this spec disagrees with the migration doc, see "Migration-doc overrides" at the top.

```sql
-- agents
ALTER TABLE agents ADD COLUMN agent_card_url TEXT;             -- nullable; CHECK below
ALTER TABLE agents ADD COLUMN agent_card_cached JSONB;
ALTER TABLE agents ADD COLUMN agent_card_fetched_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN agent_card_signed BOOLEAN DEFAULT false;
ALTER TABLE agents ADD COLUMN agent_card_signature_verified BOOLEAN DEFAULT false;
ALTER TABLE agents ADD COLUMN mode TEXT NOT NULL DEFAULT 'pull'
  CHECK (mode IN ('pull', 'push'));
ALTER TABLE agents ADD COLUMN advertise_via_mdns BOOLEAN DEFAULT true;
ALTER TABLE agents ADD COLUMN tags TEXT[] DEFAULT '{}';
ALTER TABLE agents ADD COLUMN last_polled_at TIMESTAMPTZ;       -- pull heartbeat
ALTER TABLE agents ADD COLUMN mdns_advertised_at TIMESTAMPTZ;   -- mDNS publisher liveness
ALTER TABLE agents ADD COLUMN noindex BOOLEAN DEFAULT false;
ALTER TABLE agents ADD COLUMN tombstoned_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN forced_takedown_at TIMESTAMPTZ;

-- card-or-pull constraint
-- IMPORTANT migration order:
--   1. Run the ALTER TABLE that adds `mode` (with DEFAULT 'pull') first.
--   2. Backfill: any historical row with `agent_card_url IS NOT NULL` MUST be
--      flipped to mode='push' before the CHECK is added.
--      (Today's scheduler-demo creates pull-style rows with no card URL, so the
--       default of 'pull' is correct for them — no flip needed.)
--   3. Then add the CHECK in a separate transaction.
ALTER TABLE agents ADD CONSTRAINT agents_mode_card_consistency CHECK (
  (mode = 'push' AND agent_card_url IS NOT NULL) OR
  (mode = 'pull' AND agent_card_url IS NULL)
);

-- partial unique slug index (only live agents)
CREATE UNIQUE INDEX idx_agents_account_slug_live
  ON agents(account_id, slug)
  WHERE tombstoned_at IS NULL;

-- accounts
ALTER TABLE accounts ADD COLUMN tombstoned_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN forced_takedown_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN verified_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN verification_method TEXT
  CHECK (verification_method IN ('dns_txt', 'google_workspace'));
ALTER TABLE accounts ADD CONSTRAINT accounts_verified_method_consistency CHECK (
  verified_at IS NULL OR verification_method IS NOT NULL
);

-- partial unique account slug
CREATE UNIQUE INDEX idx_accounts_slug_live
  ON accounts(slug)
  WHERE tombstoned_at IS NULL;

-- slug aliases (renames)
CREATE TABLE slug_aliases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scope TEXT NOT NULL CHECK (scope IN ('account', 'agent')),
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    old_slug TEXT NOT NULL,
    new_slug TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    UNIQUE (scope, account_id, old_slug),
    CHECK (expires_at > created_at)
);
CREATE INDEX idx_slug_aliases_active
  ON slug_aliases(scope, account_id, old_slug)
  WHERE expires_at > now();

-- (no schema delta on grants; inbox-bridge parking timeout is a server-
--  wide config in ~/.chakramcp/server.toml: inbox_bridge.parking_timeout_s
--  default 60 s, bounded [5, 600]. Per-grant or per-capability overrides
--  can be added additively if a concrete need surfaces.)

-- friendships forward-compat
ALTER TABLE friendships ADD COLUMN provenance JSONB DEFAULT '{}';

-- capabilities (from migration doc)
ALTER TABLE capabilities ADD COLUMN synced_from_card BOOLEAN DEFAULT true;
ALTER TABLE capabilities ADD COLUMN card_skill_id TEXT;
ALTER TABLE capabilities ADD COLUMN card_skill_name TEXT;
ALTER TABLE capabilities ADD COLUMN last_synced_at TIMESTAMPTZ;
```

### Indexes

```sql
CREATE INDEX idx_agents_last_polled_at
  ON agents(last_polled_at) WHERE tombstoned_at IS NULL;
CREATE INDEX idx_agents_tags_gin
  ON agents USING GIN (tags);
CREATE INDEX idx_capabilities_output_schema_jsonb
  ON capabilities USING GIN (output_schema jsonb_path_ops);
CREATE INDEX idx_capabilities_input_schema_jsonb
  ON capabilities USING GIN (input_schema jsonb_path_ops);
-- full-text search defers to v1.5 / v2; no `tsvector` column in v1
```

The `output_schema` GIN index supports JSONB containment (`@>`). Full JSON Schema validation in queries is v2.

---

## Open questions / explicitly deferred

| Question | Disposition |
|---|---|
| Cross-network friendship import | v2; schema accommodates via `friendships.provenance` |
| Reputation signals from `relay_log` | v2 |
| Compliance attestation filters | v2; schema accommodates via `agents.tags` |
| Geographic / latency-aware routing | v3+ |
| Vanity flat slugs | future feature |
| Subscription-style alerts | v2 |
| Industry-vertical tag UI surfacing | v1.5 |
| Streaming SSE proxy under mid-stream policy expiry | separate doc |
| Full JSON Schema validation in capability search | v2 (v1 = JSONB containment only) |
| Self-serve trademark takedown form | v2 (v1 = manual review email) |

---

## Appendix: example flows

### Stranger fetches Alice's card

```
GET https://chakramcp.com/agents/acme-corp/alice-scheduler/.well-known/agent-card.json
→ 200 OK
  Content-Type: application/json
  Cache-Control: public, max-age=300

  { "name": "Alice Scheduler",
    "description": "Returns 30-min slots in the next N days.",
    "supported_interfaces": [
      { "url": "https://chakramcp.com/agents/acme-corp/alice-scheduler/a2a/jsonrpc",
        "protocol_binding": "JSONRPC",
        "protocol_version": "0.3" }
    ],
    "version": "0.1.0",
    "capabilities": { "streaming": false, "push_notifications": false },
    "security_schemes": {
      "chakramcp_bearer": {
        "http": {
          "scheme": "Bearer",
          "bearer_format": "JWT",
          "description": "ChakraMCP-issued bearer token (API key or OAuth-issued JWT)."
        }
      }
    },
    "security_requirements": [ { "chakramcp_bearer": [] } ],
    "default_input_modes": ["application/json"],
    "default_output_modes": ["application/json"],
    "skills": [
      { "id": "<capability-uuid>",
        "name": "propose_slots",
        "description": "Return a list of available 30-minute slots in the next N days.",
        "tags": [],
        "examples": [],
        "input_modes": ["application/json"],
        "output_modes": ["application/json"] }
    ],
    "signatures": [
      { "protected": "<base64url JWS protected header with alg=EdDSA, kid=relay-2026-04>",
        "signature": "<base64url signature bytes>" }
    ]
  }
```

The card is **fully wire-compatible with canonical A2A v0.3**. Generic A2A clients (Google's reference SDK, openclaw-a2a-gateway) parse it without ChakraMCP-specific knowledge. Capability JSON Schemas do NOT live in the card — A2A's `AgentSkill` has no `inputSchema`/`outputSchema` fields. Schemas are exposed via our REST endpoint at `/v1/discovery/agents/<account>/<slug>/capabilities` (see "Discovery API surface").

### Stranger calls without auth

```
POST https://chakramcp.com/agents/acme-corp/alice-scheduler/a2a/jsonrpc
{ "jsonrpc": "2.0", "method": "SendMessage", ... }
→ 401
  { "jsonrpc": "2.0", "id": ..., "error": {
      "code": -32000, "message": "authentication required",
      "data": { "code": "chk.auth.missing" } } }
```

Client resolves "chk.auth.missing" against `chakramcp.com/.well-known/error-codes.json` to learn the signup URL (which can be updated server-side without redeploying clients).

### Pull-mode invocation lifecycle

```
t=0      Bob's SDK: invoke_and_wait(grant_id, input)
         → POST /agents/acme-corp/alice-scheduler/a2a/jsonrpc (SendMessage)
         → relay: auth ok, policy ok, target=pull, park
         → returns A2A Task with state=working, id=task_42

t=0.3    Alice's SDK polls /v1/inbox?agent_id=alice-scheduler
         → relay returns parked call payload (with friendship_context, grant_context)
         → SDK invokes user's handler

t=2.1    Alice's SDK: POST /v1/invocations/task_42/result {output}
         → relay translates to A2A Task.completed
         → caller's GetTask resolves

t=2.3    Bob's SDK: invoke_and_wait returns the output
```

### LAN peer browses mDNS

```
$ dns-sd -B _a2a._tcp local.
12:00:01.123 alice-scheduler@kaustav-mbp-a3f9  _a2a._tcp.local.

$ dns-sd -L "alice-scheduler@kaustav-mbp-a3f9" _a2a._tcp local.
12:00:02.456 alice-scheduler@kaustav-mbp-a3f9._a2a._tcp.local. can be reached at
             kaustav-mbp.local.:8090
             v=1 account=acme-corp agent=alice-scheduler
             card=/agents/acme-corp/alice-scheduler/.well-known/agent-card.json
             endpoint=/agents/acme-corp/alice-scheduler/a2a/jsonrpc
             mode=pull
```

---

## Implementation prerequisites

This spec assumes:

1. **SDK identity:** `chakra.invoke_and_wait()` and `inbox.serve()` survive the migration. Mappings specified above.
2. **Authentication direction:** relay-issued bearer tokens for proxied calls. **Migration doc's `relay_credentials` table is removed.**
3. **Pull-based granter model:** the relay parks A2A calls until the granter polls; granter responses release them. Specified in "Inbox bridge" section.

Each is now actually defined — no longer just an assertion.
