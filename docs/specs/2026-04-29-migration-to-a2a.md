# ChakraMCP Migration: From Proprietary Relay to A2A + MCP Trust Layer

**This document explains what changed, why it changed, and exactly how every component of the original ChakraMCP build spec maps to the new architecture that sits on top of A2A and complements MCP.**

---

> ## ⚠️ This document has been partially superseded
>
> The follow-up [discovery design spec](./2026-04-29-discovery-design.md) (rev 4, approved) overrides four decisions in this document. **Where this doc and the discovery spec disagree, the discovery spec wins.** Items overridden are marked with ~~strikethrough~~ below; see the discovery spec's "Migration-doc overrides" section for the corrected version + rationale.
>
> Quick summary of overrides:
>
> 1. ~~`agents.agent_card_url TEXT NOT NULL`~~ → nullable (pull-mode agents have no canonical card URL).
> 2. ~~Single relay endpoint `POST /relay/{account_id}/{agent_id}`~~ → two-host surface model (A2A on apex `chakramcp.com/agents/<account>/<slug>/...`; legacy ChakraMCP REST preserved on `relay.chakramcp.com/v1/...`).
> 3. ~~`relay_credentials` table holding per-target encrypted credentials~~ → relay-issued JWTs against published JWKS (no shared secrets).
> 4. ~~Multiple A2A auth schemes accepted~~ → bearer-only in v1; additive later.
>
> **Implementation order is governed by the [discovery implementation plan](./2026-04-29-discovery-implementation-plan.md) (D0–D15), not this doc's "Implementation Order" section.**

---

## Why We're Migrating

We designed ChakraMCP as a standalone relay protocol — our own wire format, our own discovery, our own task lifecycle, our own everything. Then we looked up and found that Google's A2A protocol already has 150+ organizations, production deployments at Microsoft/AWS/Salesforce/SAP/ServiceNow, Linux Foundation governance, a v1.0 spec with Signed Agent Cards, and IBM's competing ACP protocol voluntarily merged into it.

Building a competing agent communication standard against that lineup is suicide. IBM tried with ACP and surrendered.

But here's what A2A doesn't have: friendship negotiation, directional capability grants, consent modes, relay-mediated policy enforcement, acting-member audit trails, or centralized trust management. A2A tells agents *how* to talk. Nobody tells them *who's allowed* to talk, about *what*, with *whose permission*.

That's the gap. That's our product.

**The migration in one sentence:** We stop building the communication protocol and start building the trust layer that the communication protocol is missing.

---

## The Mental Model Shift

### Before (Proprietary Relay)

```
Agent A → [ChakraMCP custom protocol] → ChakraMCP Relay → [ChakraMCP custom protocol] → Agent B
```

We owned the wire format, the discovery mechanism, the task lifecycle, the event envelope, the delivery system. Agents had to speak "ChakraMCP" to use the network. If A2A existed and agents spoke A2A, they couldn't use our relay without adapting.

### After (A2A + MCP Trust Layer)

```
Agent A → [A2A protocol] → ChakraMCP Relay (policy check) → [A2A protocol] → Agent B
                                    │
                              Uses MCP for
                           tool/data access
                            within agents
```

Agents speak A2A to each other. The relay is a **policy-enforcing proxy** that intercepts A2A calls, checks trust rules, and forwards authorized calls transparently. Agents also use MCP internally for tool and data access. ChakraMCP doesn't touch the MCP layer — we operate above it at the agent-to-agent trust level.

**Analogy:** A2A is HTTP. MCP is database drivers. ChakraMCP is Cloudflare — we sit in the middle of HTTP traffic, enforce security policies, and the endpoints don't need to know we're there.

---

## What Stays the Same

The entire trust and policy model survives the migration unchanged. This is the core IP. None of it exists in A2A.

| Component | Status | Notes |
|---|---|---|
| Accounts | **Unchanged** | A2A has no organizational identity model |
| Members | **Unchanged** | A2A has no acting-member concept |
| Friendships | **Unchanged** | A2A has no relationship model |
| Access Proposals | **Unchanged** | A2A has no negotiation protocol |
| Counteroffers | **Unchanged** | A2A has no counteroffer mechanism |
| Directional Grants | **Unchanged** | A2A has no capability-scoped, time-bound, rate-limited grants |
| Consent Records | **Unchanged** | A2A has no consent modes (per-invocation, time-boxed, persistent) |
| Policy Enforcement (10-step check) | **Unchanged** | A2A is peer-to-peer with no intermediary enforcement |
| Acting Member Context | **Unchanged** | A2A doesn't track humans acting through agents |
| Audit Log | **Unchanged** | A2A has no centralized audit |
| ChakraMCP Event System | **Unchanged** | Trust events (proposals, consent requests, grant changes) stay as-is |
| JWT Auth for Management API | **Unchanged** | Our control plane auth is independent of A2A auth |
| Webhook Delivery for Trust Events | **Unchanged** | Event delivery for proposals/consent/grants stays as-is |

**Database tables that don't change at all:** accounts, members, friendships, access_proposals, grants, consent_records, events, audit_log. Their schemas, indexes, and queries remain identical.

---

## What Changes

### 1. Agent Registration

**Before:** Agents registered with ChakraMCP by providing their metadata, capabilities, and delivery settings directly. ChakraMCP was the source of truth for what an agent could do.

**After:** Agents register with ChakraMCP by providing their **A2A Agent Card URL.** ChakraMCP fetches the Agent Card, indexes the capabilities it declares, and adds a policy overlay (visibility, consent modes, grant requirements) on top. The Agent Card is the source of truth for what an agent can do. ChakraMCP is the source of truth for who's allowed to use it.

**Schema change — agents table:**

```sql
-- ADD these columns
-- ⚠️ OVERRIDE: agent_card_url MUST be nullable (pull-mode agents).
-- See discovery spec; the line below is no longer the canonical schema.
-- ~~ALTER TABLE agents ADD COLUMN agent_card_url TEXT NOT NULL;~~
ALTER TABLE agents ADD COLUMN agent_card_url TEXT;   -- canonical: nullable
ALTER TABLE agents ADD COLUMN agent_card_cached JSONB;
ALTER TABLE agents ADD COLUMN agent_card_fetched_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN agent_card_signed BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE agents ADD COLUMN agent_card_signature_verified BOOLEAN NOT NULL DEFAULT false;

-- REMOVE these columns (now sourced from Agent Card)
-- display_name stays (can be overridden or synced from card)
-- description stays (can be overridden or synced from card)
-- tags stays (can be merged with card tags)

-- delivery_polling_enabled STAYS (for ChakraMCP trust events, NOT A2A calls)
-- delivery_webhook_url STAYS (for ChakraMCP trust events, NOT A2A calls)
-- webhook_secret_current STAYS
-- webhook_secret_previous STAYS
-- webhook_secret_previous_expires_at STAYS
```

**Key distinction:** The `delivery_*` fields on the agent are for **ChakraMCP trust events** (proposals, consent requests, grant changes). They are NOT for A2A task delivery. A2A calls flow through the relay proxy, not through the event system.

**New registration flow:**

```
1. Creator calls POST /v1/agents with:
   - agent_id
   - agent_card_url (e.g., "https://myagent.com/.well-known/agent-card.json")
   - policy_default_visibility ("public" or "friend-gated")
   - Optional: delivery settings for trust events

2. ChakraMCP fetches the Agent Card from agent_card_url
3. ChakraMCP verifies the Signed Agent Card signature (if present, A2A v1.0+)
4. ChakraMCP parses capabilities/skills from the Agent Card
5. ChakraMCP creates capability records with policy overlay defaults
6. ChakraMCP stores the cached Agent Card
7. Agent is now discoverable and can participate in trust negotiation
```

**New endpoint:**
- `POST /v1/agents/{agent_id}/sync-card` — Force re-fetch and re-index the Agent Card

**Background job:**
- Periodic Agent Card refresh (every 1 hour) for all active agents
- On capability diff: add new, mark removed, update descriptions
- Never overwrite ChakraMCP policy overlay (visibility, consent modes) — those are user-configured

---

### 2. Capabilities Table

**Before:** Capabilities were manually declared during agent registration. ChakraMCP was the sole source.

**After:** Capabilities are **imported from the A2A Agent Card** and enriched with ChakraMCP policy metadata. The Agent Card declares what the agent can do. ChakraMCP declares who's allowed to use each capability and under what rules.

**Schema change — capabilities table:**

```sql
-- ADD these columns
ALTER TABLE capabilities ADD COLUMN synced_from_card BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE capabilities ADD COLUMN card_skill_id TEXT;         -- original skill ID from Agent Card
ALTER TABLE capabilities ADD COLUMN card_skill_name TEXT;       -- original skill name from Agent Card
ALTER TABLE capabilities ADD COLUMN last_synced_at TIMESTAMPTZ;

-- RENAME 'kind' options
-- A2A uses 'skill' as the primary unit. Keep 'tool' and 'workflow' for ChakraMCP policy,
-- add 'skill' as an option for A2A-native capabilities
ALTER TABLE capabilities DROP CONSTRAINT capabilities_kind_check;
ALTER TABLE capabilities ADD CONSTRAINT capabilities_kind_check
    CHECK (kind IN ('tool', 'workflow', 'skill'));
```

**The two-layer model:**

```
A2A Agent Card says:          ChakraMCP policy overlay says:
  skill: "trip-plan"            visibility: "friend-gated"
  description: "Plans trips"    consent_mode: "per-invocation"
  input schema: {...}           requires_admin: true
                                constraint_schema: {max_duration: 60}
```

A2A tells you *what the capability is.* ChakraMCP tells you *who can use it and how.*

---

### 3. Discovery

**Before:** Discovery searched ChakraMCP's own agent and capability registry.

**After:** Discovery searches a **merged index** of A2A Agent Card data and ChakraMCP trust metadata. A user searching for "trip planning" finds agents based on Agent Card descriptions and skills, but also sees whether they need friendship, which capabilities are friend-gated, and what consent modes apply.

**Endpoint changes:**

`GET /v1/discovery/agents` now returns:

```json
{
    "agent_id": "travel-planner",
    "account_id": "acct_orbit",
    "display_name": "Travel Planner",
    "description": "Plans and books business travel",
    "agent_card_url": "https://travel.example.com/.well-known/agent-card.json",
    "agent_card_signed": true,
    "capabilities": [
        {
            "id": "trip-plan.run",
            "kind": "skill",
            "description": "Creates a full travel itinerary",
            "source": "agent_card",
            "visibility": "friend-gated",
            "consent_mode": "per-invocation",
            "requires_friendship": true,
            "requires_grant": true,
            "your_grant_status": "none"
        }
    ],
    "friendship_status": "none",
    "tags": ["travel", "booking", "enterprise"]
}
```

The discovery response now tells the requester exactly what trust steps are needed before they can use each capability: do they need friendship? Do they need a grant? Do they need consent? What's their current status?

---

### 4. Relay Execution (THE BIG CHANGE)

**Before:** The relay received ChakraMCP-native requests, checked policy, and forwarded ChakraMCP-native calls to the target agent.

**After:** The relay receives **standard A2A JSON-RPC requests,** checks policy, and forwards **standard A2A JSON-RPC calls** to the target agent. The relay is a transparent policy-enforcing proxy.

**Old relay flow:**

```
Agent A → ChakraMCP request → Relay → policy check → ChakraMCP request → Agent B
```

**New relay flow:**

```
Agent A → A2A SendMessage (via relay endpoint) → Relay → policy check →
    → fetch Agent B's Agent Card for real endpoint →
    → A2A SendMessage (to Agent B's real endpoint) → Agent B
    → Agent B's A2A response → back through relay → Agent A
```

**New relay endpoint:**

> ⚠️ **OVERRIDDEN** by the discovery spec — the canonical A2A endpoint is now path-based on the apex host: `POST chakramcp.com/agents/<account-slug>/<agent-slug>/a2a/jsonrpc`. The UUID-based path on `relay.chakramcp.com` is reserved for the **legacy ChakraMCP REST surface** (`/v1/invoke`, `/v1/inbox`, ...) preserved for v0.1.0 SDK back-compat — it does NOT serve A2A directly. See "Two-host surface model" in the discovery spec.

```
~~POST /relay/{target_account_id}/{target_agent_id}~~
POST /agents/<account-slug>/<agent-slug>/a2a/jsonrpc      // A2A surface (canonical)
POST /agents/<account-slug>/<agent-slug>/a2a/stream       // streaming variant
```

This endpoint accepts any A2A JSON-RPC method:
- `SendMessage` — sync task execution
- `SendStreamingMessage` — streaming execution (proxied as SSE)
- `GetTask` — check task status
- `CancelTask` — cancel a running task

**The relay adds these headers to the proxied request (for audit, not for the target agent):**

```
X-ChakraMCP-Requester-Account: acct_acme
X-ChakraMCP-Requester-Agent: agt_ops_runner
X-ChakraMCP-Acting-Member: mem_maya (if present)
X-ChakraMCP-Grant-Id: grt_01JYCG0Z7NBW2A
X-ChakraMCP-Consent-Record: cnsrec_01JYCGSJKB2WWX (if applicable)
```

**Streaming support:**

For `SendStreamingMessage`, the relay:
1. Receives the SSE connection from Agent A
2. Runs policy check
3. Opens an SSE connection to Agent B's real endpoint
4. Pipes the SSE stream from Agent B back to Agent A
5. Logs the relay call on stream completion

**Authentication to target agents:**

> ⚠️ **OVERRIDDEN** by the discovery spec. The original "grant-holder supplies the target's credential" model is reversed: the relay holds **its own** Ed25519 signing key and mints short-lived per-call JWTs. Targets verify our JWTs against `chakramcp.com/.well-known/jwks.json`. The `relay_credentials` table is **removed** — DO NOT create it.
>
> Rationale: the original model gave the requester custody of the target's credential, defeating mediation. JWT-based mediation eliminates the credential-storage problem entirely. See discovery spec §"Override #3" for the full rationale + flow.

~~When the relay forwards an A2A call to Agent B, it needs to authenticate. The Agent Card declares supported auth schemes. The relay stores credentials for each registered target agent:~~

```sql
-- ⚠️ DO NOT CREATE THIS TABLE. Override by discovery spec.
-- Replaced by the relay's own JWT signing key + JWKS publication.
~~CREATE TABLE relay_credentials (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_account_id UUID NOT NULL REFERENCES accounts(id),
    target_agent_id TEXT NOT NULL,
    auth_scheme TEXT NOT NULL CHECK (auth_scheme IN ('api_key', 'bearer', 'oauth2', 'mtls')),
    credential_encrypted TEXT NOT NULL,       -- encrypted credential value
    oauth_token_url TEXT,                     -- for OAuth2 flows
    oauth_client_id TEXT,
    oauth_client_secret_encrypted TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (target_account_id, target_agent_id) REFERENCES agents(account_id, id)
);~~
```

~~These credentials are configured when a grant is established. The grant-holder (requester) provides the credentials needed to reach the target agent. The relay uses these credentials when proxying authorized calls.~~

**Replacement (canonical):** the relay holds an Ed25519 signing keypair (rotation cycle: 90 days, overlap window: 30 days). On every authorized forward, the relay mints a JWT with claims `{iss: chakramcp.com, aud: <target-agent-id>, sub: <caller-agent-id>, capability: <capability_id>, grant_id, exp: now+60s, jti}` and includes it as `Authorization: Bearer <jwt>` to the target. Targets verify against published JWKS. No shared secrets.

---

### 5. Event System

**Before:** The event system used a custom `EventEnvelope` format for all events — trust events AND capability run events.

**After:** The event system splits into two:

**ChakraMCP Trust Events** (unchanged format, unchanged delivery):
- `friendship.requested`
- `friendship.counteroffered`
- `friendship.accepted`
- `friendship.rejected`
- `grant.updated`
- `consent.requested`
- `consent.granted`
- `consent.revoked`

These use the existing ChakraMCP event envelope, delivered via polling (`GET /v1/inbox/events`) or webhook. They are NOT A2A messages. They are ChakraMCP-native trust management events.

**A2A Task Events** (new, uses A2A protocol):
- `capability.run.requested` → becomes an A2A `SendMessage` or `SendStreamingMessage` proxied through the relay
- `capability.run.cancelled` → becomes an A2A `CancelTask` proxied through the relay

Run status and results are now handled via A2A's native task lifecycle (GetTask, task status polling, push notifications) rather than ChakraMCP's custom run reporting endpoints.

**What this means for the events table:**

The events table stays but only stores ChakraMCP trust events. Remove the capability run event types from the event_type enum.

```sql
-- Event types that STAY in ChakraMCP events:
-- friendship.requested, friendship.counteroffered, friendship.accepted, friendship.rejected
-- grant.updated
-- consent.requested, consent.granted, consent.revoked

-- Event types that MOVE to A2A task lifecycle:
-- capability.run.requested → A2A SendMessage through relay
-- capability.run.cancelled → A2A CancelTask through relay
```

---

### 6. Run Tracking

**Before:** ChakraMCP had its own `capability_runs` table and custom status/result reporting endpoints.

**After:** Capability runs are A2A Tasks. A2A has a native task lifecycle with states (submitted, working, input-required, completed, failed, canceled). The relay logs every task it proxies, but it doesn't manage the task lifecycle — A2A does.

**Schema change — rename and repurpose:**

```sql
-- Rename capability_runs to relay_log
-- This table now logs every A2A call proxied through the relay
-- It does NOT manage task lifecycle (A2A does that)

CREATE TABLE relay_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Who
    requester_account_id UUID NOT NULL,
    requester_agent_id TEXT NOT NULL,
    acting_member_id UUID,
    target_account_id UUID NOT NULL,
    target_agent_id TEXT NOT NULL,
    -- What
    a2a_method TEXT NOT NULL,               -- SendMessage, SendStreamingMessage, GetTask, CancelTask
    capability_id TEXT,                      -- extracted from message content if identifiable
    a2a_task_id TEXT,                        -- A2A task ID if applicable
    -- Trust
    grant_id UUID REFERENCES grants(id),
    consent_record_id UUID REFERENCES consent_records(id),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN ('authorized', 'denied', 'consent_required')),
    denial_reason TEXT,
    -- Outcome
    target_status_code INTEGER,              -- HTTP status from target agent
    response_time_ms INTEGER,
    -- Metadata
    request_size_bytes INTEGER,
    response_size_bytes INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_relay_log_requester ON relay_log(requester_account_id, created_at DESC);
CREATE INDEX idx_relay_log_target ON relay_log(target_account_id, created_at DESC);
CREATE INDEX idx_relay_log_decision ON relay_log(policy_decision, created_at DESC);
```

**Endpoints removed:**
- ~~`POST /v1/capability-runs/{run_id}/status`~~ → Agent B reports status via A2A task lifecycle
- ~~`POST /v1/capability-runs/{run_id}/result`~~ → Agent B completes via A2A task lifecycle

**Endpoint added:**
- `GET /v1/audit/relay-log` — Query relay log (filter by account, agent, capability, time range, decision)

---

### 7. MCP Integration

**Before:** ChakraMCP treated MCP as a future transport option. MCP methods mirrored HTTP endpoints.

**After:** MCP and A2A are recognized as **different layers that ChakraMCP doesn't compete with.**

```
┌─────────────────────────────────────────────┐
│        ChakraMCP (trust & policy)            │  ← We are here
├─────────────────────────────────────────────┤
│     A2A (agent-to-agent communication)       │  ← We proxy this
├─────────────────────────────────────────────┤
│     MCP (agent-to-tools/data)                │  ← We don't touch this
└─────────────────────────────────────────────┘
```

**MCP is used internally by agents** to connect to their own tools and data sources. An agent might use MCP to query a database, call an API, or read files. ChakraMCP has no visibility into or authority over an agent's MCP connections. They're internal to the agent.

**A2A is used between agents.** When Agent A wants Agent B to do something, that's A2A. When that A2A call passes through ChakraMCP's relay, we enforce trust policies.

**ChakraMCP's MCP surface (removed):**

The original build spec included MCP methods that mirrored HTTP endpoints (`network.register_agent`, `network.list_inbox_events`, etc.). These are **removed from v1.** Reason: our management API is REST. Adding a parallel MCP control plane doubles the surface area for no user benefit at this stage. If demand emerges later, we can add an MCP server that wraps the REST API.

**What we remove:**
- All `network.*` MCP methods from the spec
- The MCP transport layer for relay calls (use A2A JSON-RPC instead)

**What we keep:**
- Awareness that agents on ChakraMCP will use MCP internally
- The capability index may include MCP-provided tools if agents declare them in their Agent Card
- Future: ChakraMCP could provide an MCP server that exposes trust management (check grant status, request consent) as MCP tools that agents can call programmatically

---

## Endpoints: Complete Before/After

### Removed Endpoints

| Old Endpoint | Why Removed |
|---|---|
| `POST /v1/capability-runs/{run_id}/status` | Task lifecycle handled by A2A natively |
| `POST /v1/capability-runs/{run_id}/result` | Task lifecycle handled by A2A natively |
| All `network.*` MCP methods | MCP control plane removed from v1 |

### Changed Endpoints

| Endpoint | What Changed |
|---|---|
| `POST /v1/agents` | Now requires `agent_card_url`. Capabilities synced from Agent Card. |
| `PATCH /v1/agents/{agent_id}` | Updates ChakraMCP policy overlay only. Agent Card fields synced separately. |
| `GET /v1/agents/{agent_id}` | Returns cached Agent Card data alongside ChakraMCP policy. |
| `GET /v1/discovery/agents` | Searches merged Agent Card + ChakraMCP index. Returns trust status. |
| `GET /v1/discovery/agents/{id}/capabilities` | Shows A2A-sourced capabilities with ChakraMCP policy overlay. |

### New Endpoints

| Endpoint | Purpose |
|---|---|
| `POST /relay/{target_account_id}/{target_agent_id}` | **THE CORE.** A2A proxy with policy enforcement. |
| `POST /relay/{target_account_id}/{target_agent_id}/stream` | Streaming A2A proxy (SSE passthrough). |
| `POST /v1/agents/{agent_id}/sync-card` | Force Agent Card re-fetch and re-index. |
| `GET /v1/audit/relay-log` | Query proxied A2A call log. |

### Unchanged Endpoints

| Endpoint | Notes |
|---|---|
| `DELETE /v1/agents/{agent_id}` | Same behavior |
| `POST /v1/agents/{agent_id}/rotate-secret` | For ChakraMCP webhook secrets, not A2A auth |
| `GET /v1/inbox/events` | Trust events only (proposals, consent, grants) |
| `POST /v1/events/{id}/ack` | Same behavior |
| `POST /v1/events/{id}/nack` | Same behavior |
| `POST /v1/proposals` | Same behavior |
| `GET /v1/proposals/inbox` | Same behavior |
| `GET /v1/proposals/outbox` | Same behavior |
| `POST /v1/proposals/{id}/accept` | Same behavior |
| `POST /v1/proposals/{id}/reject` | Same behavior |
| `POST /v1/proposals/{id}/counteroffer` | Same behavior |
| `POST /v1/consent/{id}/grant` | Same behavior |
| `POST /v1/consent/{id}/deny` | Same behavior |
| `GET /v1/consent/active` | Same behavior |
| `POST /v1/consent/{id}/revoke` | Same behavior |
| `GET /v1/grants` | Same behavior |
| `POST /v1/grants/{id}/revoke` | Same behavior |
| `GET /healthz` | Same behavior |
| `GET /readyz` | Same behavior |

---

## Implementation Order (Revised)

### Phase 1: Foundation (unchanged)

Axum server, PostgreSQL, config, tracing, error handling, migrations, health checks.

### Phase 2: Agent Registration with Agent Card Sync (changed)

1. `POST /v1/agents` — now accepts `agent_card_url`
2. Agent Card fetcher — HTTP GET, parse A2A AgentCard JSON
3. Signed Agent Card verification (if signature present)
4. Capability indexing from Agent Card with policy overlay
5. `GET /v1/agents/{agent_id}` — returns merged data
6. `PATCH /v1/agents/{agent_id}` — updates policy overlay only
7. `POST /v1/agents/{agent_id}/sync-card` — manual re-sync
8. Background Agent Card refresh job (hourly)
9. JWT auth middleware

**Checkpoint: agents register by pointing to their A2A Agent Card. ChakraMCP fetches, indexes, and overlays policy.**

### Phase 3: Discovery (changed)

1. `GET /v1/discovery/agents` — full-text search on merged Agent Card + ChakraMCP data
2. `GET /v1/discovery/agents/{id}/capabilities` — capabilities with trust status

**Checkpoint: agents are discoverable with trust context.**

### Phase 4: Proposals and Friendships (unchanged)

Same as original build spec. No A2A dependency here — this is pure ChakraMCP trust management.

**Checkpoint: accounts can negotiate access.**

### Phase 5: Trust Event System (unchanged)

Same as original build spec. Trust events (proposals, consent, grants) use ChakraMCP's own event system.

**Checkpoint: agents receive trust events via polling.**

### Phase 6: Consent (unchanged)

Same as original build spec.

**Checkpoint: sensitive capabilities require approval.**

### Phase 7: THE RELAY (new — replaces old relay + run tracking)

This is the new core. Build in this order:

1. **A2A types** — Rust structs for AgentCard, Task, Message, Artifact, Part, JSON-RPC request/response
2. **A2A client** — HTTP client that sends A2A JSON-RPC calls to target agents
3. **Relay credentials table** — store auth credentials for target agents
4. **Policy check function** — the 10-step authorization (unchanged logic)
5. **Relay endpoint** — `POST /relay/{target_account_id}/{target_agent_id}`
   - Parse incoming A2A JSON-RPC request
   - Extract caller identity from ChakraMCP JWT
   - Run policy check
   - If authorized: fetch target Agent Card for real endpoint, forward A2A call, return response
   - If denied: return structured denial
   - If consent needed: create consent request event, return waiting status
6. **Relay log** — write every proxied call to relay_log table
7. **Streaming relay** — `POST /relay/.../stream` for SSE passthrough
8. **Audit endpoint** — `GET /v1/audit/relay-log`

**Checkpoint: agents can execute A2A calls through the relay with full trust enforcement.**

### Phase 8: Webhook Delivery for Trust Events (unchanged)

Same as original. ChakraMCP trust events pushed to agent webhook endpoints.

### Phase 9: Hardening (unchanged)

Rate limiting, request ID propagation, input validation, pagination, graceful shutdown, integration tests.

---

## Migration Checklist

For anyone working on the codebase, here's the complete list of changes:

### Database

> ⚠️ Two checklist items are overridden — see discovery spec for the full canonical schema.

- [ ] Add `agent_card_url` (**nullable** per override), `agent_card_cached`, `agent_card_fetched_at`, `agent_card_signed`, `agent_card_signature_verified` to agents table
- [ ] Add `synced_from_card`, `card_skill_id`, `card_skill_name`, `last_synced_at` to capabilities table
- [ ] Add `skill` to capabilities.kind check constraint
- [ ] ~~Create `relay_credentials` table~~ **SKIP** — replaced by relay JWT signing keys (see Override #3)
- [ ] Create `relay_log` table (replaces capability_runs for logging purposes)
- [ ] Drop `capability_runs` table (task lifecycle is now A2A-native)
- [ ] Remove `capability.run.requested` and `capability.run.cancelled` from events.event_type
- [ ] **Add discovery-spec deltas** (mode column, slug_aliases, accounts.tombstoned_at, friendships.provenance, GIN indexes, etc.) — see `2026-04-29-discovery-design.md` §"Schema deltas"

### New Rust Modules

- [ ] `src/a2a/types.rs` — A2A protocol types (AgentCard, Task, Message, etc.)
- [ ] `src/a2a/agent_card.rs` — fetch, parse, verify Agent Cards
- [ ] `src/a2a/client.rs` — A2A JSON-RPC HTTP client
- [ ] `src/a2a/proxy.rs` — transparent A2A request forwarding
- [ ] `src/routes/relay.rs` — relay endpoint with policy check + proxy
- [ ] `src/db/relay_log.rs` — relay log queries
- [ ] ~~`src/db/relay_credentials.rs` — credential storage for target agents~~ **SKIP** — replaced by `src/jwt/keys.rs` + `src/jwt/mint.rs` (relay-issued JWTs against published JWKS; see discovery implementation plan §D5)

### Modified Rust Modules

- [ ] `src/routes/agents.rs` — registration now accepts agent_card_url, triggers Agent Card fetch
- [ ] `src/routes/discovery.rs` — search returns merged Agent Card + ChakraMCP data with trust status
- [ ] `src/models/agent.rs` — add Agent Card fields
- [ ] `src/models/capability.rs` — add sync fields

### Removed Rust Modules

- [ ] `src/routes/runs.rs` — run status/result endpoints removed
- [ ] `src/db/runs.rs` — capability runs DB queries removed
- [ ] `src/models/run.rs` — run model removed

### Removed from Spec

- [ ] All `network.*` MCP methods
- [ ] `POST /v1/capability-runs/{run_id}/status`
- [ ] `POST /v1/capability-runs/{run_id}/result`
- [ ] Custom event types for capability runs

---

## A2A Protocol Reference

When implementing A2A types and client code, refer to:

- **Spec:** https://a2a-protocol.org/latest/specification/
- **Agent Card format:** JSON at `/.well-known/agent-card.json` per RFC 8615
- **Wire protocol:** JSON-RPC 2.0 over HTTP
- **Core methods:** SendMessage, SendStreamingMessage, GetTask, CancelTask, SetTaskPushNotificationConfig
- **Task states:** submitted, working, input-required, auth-required, completed, failed, canceled, rejected
- **Auth schemes in Agent Card:** apiKey, http (bearer), oauth2, openIdConnect
- **Signed Agent Cards (v1.0+):** cryptographic signature for domain verification

### Minimum A2A Types to Implement

```rust
// Agent Card (fetched from target agents)
struct A2AAgentCard {
    name: String,
    description: Option<String>,
    url: String,                    // agent's real A2A endpoint
    version: Option<String>,
    skills: Option<Vec<A2ASkill>>,
    authentication: Option<Vec<A2AAuthScheme>>,
    capabilities: Option<A2ACapabilities>,
}

// JSON-RPC request (what we receive and forward)
struct JsonRpcRequest {
    jsonrpc: String,                // always "2.0"
    method: String,                 // SendMessage, GetTask, etc.
    params: serde_json::Value,      // method-specific params
    id: serde_json::Value,          // request ID
}

// JSON-RPC response (what we proxy back)
struct JsonRpcResponse {
    jsonrpc: String,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
    id: serde_json::Value,
}
```

For the relay proxy, we don't need to deeply parse A2A message content. We receive a JsonRpcRequest, run our policy check using metadata (requester identity, target agent, capability), and forward the raw request to the target. We are a **transparent proxy** — we don't rewrite A2A payloads.

---

## Summary

| Aspect | Before | After |
|---|---|---|
| Wire protocol | Custom ChakraMCP | A2A JSON-RPC 2.0 (we proxy) |
| Discovery | ChakraMCP-native registry | Agent Cards + ChakraMCP policy index |
| Task lifecycle | Custom run tracking | A2A native (submitted→working→completed) |
| Trust model | ChakraMCP-native | ChakraMCP-native (unchanged — this is our product) |
| MCP integration | Mirrored control plane | Removed from v1 (agents use MCP internally) |
| Relay role | Custom protocol mediator | A2A policy-enforcing proxy |
| Competitive posture | Alternative to A2A | Value-added layer on top of A2A |
| Market position | Protocol competitor | Trust infrastructure provider |

The trust model — friendships, grants, consent, enforcement, audit — is the product. Everything else is plumbing. The migration changes the plumbing from proprietary to standards-based. The product stays exactly the same.
