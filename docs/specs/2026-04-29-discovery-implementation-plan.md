# Discovery — implementation plan

**Date:** 2026-04-29 · **Status:** Draft, pending user sign-off
**Spec:** `2026-04-29-discovery-design.md` (rev 4, approved)
**Migration context:** `chakramcp-migration-to-a2a.md` (Phases 2/3/7)

This plan turns the discovery design into sequenced work. Each phase is a sensible commit boundary that leaves the system in a working state. The existing `examples/scheduler-demo/` is the integration test at every phase boundary — it must keep working end-to-end (or be cleanly gated behind the same feature flag as new code).

---

## Phasing strategy

- **Single feature flag** `DISCOVERY_V2` (server-side env var; SDK-side build-time constant) gates the new behavior. Default `false` until phase D14. Lets us ship code without flipping production over.
- **Schema is forward-compatible.** New columns are nullable or have safe defaults. Old code keeps working against the new schema.
- **Two-host surface model is implemented from D2 onward.** Legacy `relay.chakramcp.com/v1/...` routes stay intact throughout; new `chakramcp.com/agents/<account>/<slug>/...` routes are added.
- **Migration doc patch lands before D1** so anyone reading either doc sees the same story.

---

## Phase D0 — Migration doc patch + plan acceptance

**What:** Edit `chakramcp-migration-to-a2a.md` to strikethrough the four items the discovery spec overrides (`relay_credentials` table, `agent_card_url NOT NULL`, UUID-based relay endpoint, target-credential storage). Add a banner pointing at the discovery spec for the corrected story.

**Files:** `docs/chakramcp-migration-to-a2a.md` (or wherever it ends up living in the repo).

**Why first:** prevents implementers from following the wrong checklist.

**Commit:** "Patch migration doc: strikethrough items overridden by discovery spec rev 4"

**Risk:** none. Doc-only.

---

## Phase D1 — Schema migrations

**What:** Land all schema deltas from the discovery spec, in safe order, behind no flag (the schema is forward-compatible).

**Files:**

- `backend/migrations/0009_a2a_discovery_schema.sql` (or whichever number is next)
- `backend/migrations/0010_a2a_discovery_check_constraint.sql` — separated so the CHECK lands AFTER backfill in step 3

**Migration order (in two transaction-bounded files):**

1. **0009 — additive columns + indexes:**
   - `agents`: add `agent_card_url`, `agent_card_cached`, `agent_card_fetched_at`, `agent_card_signed`, `agent_card_signature_verified`, `mode` (default `'pull'`), `advertise_via_mdns`, `tags`, `last_polled_at`, `mdns_advertised_at`, `noindex`, `tombstoned_at`, `forced_takedown_at`
   - `accounts`: add `tombstoned_at`, `forced_takedown_at`, `verified_at`, `verification_method`, plus `verified_method_consistency` CHECK
   - `slug_aliases`: create table
   - `friendships`: add `provenance JSONB`
   - `capabilities`: add `synced_from_card`, `card_skill_id`, `card_skill_name`, `last_synced_at`
   - Partial unique indexes: `idx_agents_account_slug_live`, `idx_accounts_slug_live`
   - GIN indexes: `idx_capabilities_output_schema_jsonb`, `idx_capabilities_input_schema_jsonb`, `idx_agents_tags_gin`
   - `idx_agents_last_polled_at`
   - `idx_slug_aliases_active`

2. **Backfill (data migration in same file as 0009):**
   - For any row where `agent_card_url IS NOT NULL`, set `mode = 'push'`. (Today's scheduler-demo agents have NULL — they stay `'pull'` by default. Verified.)

3. **0010 — CHECK constraint:**
   - `agents_mode_card_consistency` (after backfill)

**Verification:** `cargo test -p chakramcp-relay` passes; `task db:up` + scheduler-demo still works against new schema.

**Commit:** "Schema migrations for A2A discovery (additive)"

**Risk:** Low. Constraints are additive. Backfill is small (today: zero rows match the predicate).

---

## Phase D2 — Agent Card service (the foundation)

**What:** Server can synthesize, fetch, sign, and serve Agent Cards. This is the load-bearing module everything downstream depends on.

**New Rust modules:**

```
backend/relay/src/agent_card/
├── mod.rs            -- pub re-exports + AgentCard types
├── types.rs          -- A2A-spec card structs (skill, auth, capabilities, signature)
├── synthesizer.rs    -- pull-mode synthesis from registration data
├── fetcher.rs        -- push-mode fetch with ETag/Cache-Control clamping (max 60min)
├── signer.rs         -- Ed25519 signing with kid rotation
├── jwks.rs           -- /.well-known/jwks.json publication
└── refresh_job.rs    -- background loop that re-fetches push cards (per-row SKIP LOCKED lease)
```

**New HTTP routes:** (gated by `DISCOVERY_V2` feature flag — fail closed when off)

- `GET /agents/<account>/<slug>/.well-known/agent-card.json` — serve cached card
- `GET /.well-known/jwks.json` — relay's signing keys
- `GET /.well-known/error-codes.json` — error catalog (stub for now; D12 fills in)

**Refresh job:**
- Reuses existing background-task pattern in `backend/relay/src/jobs/`
- Per-row lease via `UPDATE agents SET last_card_fetched_at = now() WHERE id = ... AND ...` with `SKIP LOCKED`
- Cap fetch to once per 60 min regardless of upstream `Cache-Control`

**Tests:**
- Unit: synthesis from registration fixture; signature verification roundtrip; ETag-aware fetch with mocked HTTP server
- Integration: `GET /agents/.../agent-card.json` for a pull-mode agent returns synthesized signed card; same for push-mode against a fixture upstream

**Commit boundary:** one commit per module, final integration commit ties them in.

**Risk:** Medium. Signing key management and JWKS rotation need to be right; bugs here are public-facing.

---

## Phase D3 — A2A endpoint scaffolding (501 stubs)

**What:** Wire up the routes that will accept A2A JSON-RPC, but return 501 Not Implemented for now. Lets us test card-published URLs resolve to *something* before the real proxy lands.

**New routes (behind `DISCOVERY_V2`):**
- `POST /agents/<account>/<slug>/a2a/jsonrpc` → 501
- `POST /agents/<account>/<slug>/a2a/stream` → 501

**Files:** `backend/relay/src/handlers/a2a.rs`

**Why this phase exists:** D2 publishes cards with a `url` field pointing here. If callers try to actually call, they should get a clean 501 with `data.code = chk.not_implemented_yet` rather than 404.

**Tests:** route exists, returns 501 with structured error.

**Commit:** "A2A endpoint stubs (501 until D5)"

**Risk:** None.

---

## Phase D4 — Auth + policy decision tree

**What:** Implement bearer JWT validation and the 10-step policy decision (friendship → grant → consent → tombstone → unreachable). No forwarding yet — just the gate.

**New module:** `backend/relay/src/policy/decision.rs`

**Behavior:**
- Validate `Authorization: Bearer <jwt>`. ChakraMCP API keys (`ck_…`) and OAuth-issued JWTs both resolve to a caller identity (existing logic; reuse).
- Run the decision tree from spec section "Decision tree (auth + policy)."
- On any failure branch, return the structured A2A error with stable `data.code`.
- On pass, return `Authorization: Pass` with the resolved (caller agent, target capability, grant_id) tuple.

**Wire into D3 stub:** the stub now runs the decision; on pass, still returns 501 (forwarding lands in D5).

**Tests:**
- Unit: each branch of the decision tree (no auth → -32000, invalid token → -32001, no friendship → -32002, no grant → -32003, etc.)
- Integration: stranger calls a public agent without auth → 401 with `data.code = chk.auth.missing`

**Commit:** "Auth + policy decision tree for A2A relay endpoint"

**Risk:** Medium. The decision tree branches are the policy product — bugs here have direct trust implications. Heavy test coverage required.

---

## Phase D5 — Relay-issued JWT minter + forwarder (Override #3)

**What:** Now the route actually forwards. For push targets, mint a JWT and POST the A2A request to the target's canonical URL. For pull targets, park in the inbox bridge.

**New modules:**

```
backend/relay/src/
├── jwt/
│   ├── mod.rs
│   ├── mint.rs       -- mint per-call short-lived JWTs (60s exp)
│   ├── keys.rs       -- key storage + rotation (90d cycle, 30d overlap)
│   └── jwks.rs       -- already in D2; just add new keys here
├── forwarder/
│   ├── mod.rs
│   └── push.rs       -- HTTP POST to target's canonical URL with our JWT
└── inbox_bridge/
    ├── mod.rs
    ├── park.rs       -- write parked-call row to relay_log
    ├── poll.rs       -- granter SDK polls; return parked calls in legacy Invocation shape with additive fields
    └── release.rs    -- granter posts response; translate to A2A Task.completed/failed
```

**Schema reuse:** `relay_log` table from migration doc Phase 7. Discovery spec doesn't change its shape.

**JWT signing key:** Ed25519 keypair, generated on first server boot, persisted to `<state-dir>/relay-signing-keys.json` (encrypted at rest with relay master key from env). JWKS endpoint (D2) starts publishing the public half.

**Push forwarder:**
- Resolves target's actual A2A endpoint from `agents.agent_card_url` (push) or fails (`-32005` unreachable).
- Mints JWT with claims (iss, aud, sub, capability, grant_id, exp, jti).
- POSTs the original A2A request body verbatim with `Authorization: Bearer <our-jwt>`.
- Streams response back to the caller (sync) or pipes SSE (streaming).

**Inbox bridge:** spec section "Inbox bridge (pull-mode contract)" is the contract. Park, poll, release. Timeouts via global config `inbox_bridge.parking_timeout_s` (default 60s).

**Wire into D4:** the decision tree's "pass" branch now invokes the right forwarder.

**Tests:**
- Unit: JWT minting with claim shape; key rotation overlap behavior
- Integration: end-to-end call against a test push agent (mocks our JWT verifier); end-to-end park + poll + release for a pull agent
- Regression: scheduler-demo runs end-to-end against the new flow with `DISCOVERY_V2=true`

**Commit boundary:** four commits — keys + JWKS, mint, push forwarder, inbox bridge.

**Risk:** **High.** This is the core. JWT key management leaks here are the worst-case failure (anyone with our key can impersonate any caller). Encryption at rest, secure key generation, no logging of key material — the usual hardening. Recommend a security review before D14 flag flip.

---

## Phase D6 — SDK preservation shims

**What:** The published v0.1.0 TS + Python SDKs keep working. Their wire calls now translate to A2A internally.

**TS SDK (`sdks/typescript/src/`):**
- `invoke_and_wait()` posts to `/v1/invoke` (preserved); the relay translates to A2A `SendMessage` internally before the policy + park/forward path. **No SDK code change required** — the wire shape stays the same; the relay's translation is server-side.
- `inbox.serve()` polls `/v1/inbox` (preserved); the response payload now includes the additive `parked_a2a_request`, `parked_at`, `parking_deadline_at` fields. SDK signature unchanged; advanced users can read the new fields off the Invocation dict.
- Fallback: if a future SDK release wants to talk A2A natively, it can target `chakramcp.com/agents/<account>/<slug>/a2a/jsonrpc` directly.

**Python SDK (`sdks/python/src/chakramcp/`):**
- Identical to TS: no signature changes, wire preserved.

**Files actually touched:**
- TS: `sdks/typescript/src/types.ts` add the additive fields to the `Invocation` type.
- Python: `sdks/python/src/chakramcp/_types.py` same.
- Both: `CHANGELOG.md` entry noting v0.2.0 will bring A2A-native paths but v0.1.x SDK keeps working unchanged.

**Server-side translation work (the actual code):**
- `backend/relay/src/handlers/legacy_invoke.rs` (rename existing `invoke.rs` if needed): on `POST /v1/invoke`, build an A2A `SendMessage` body from the legacy params and call into the same policy/forward path D5 set up.
- `backend/relay/src/handlers/legacy_inbox.rs`: on `GET /v1/inbox`, query parked calls from relay_log and shape into the legacy Invocation list with new additive fields.
- `backend/relay/src/handlers/legacy_invocation_result.rs`: on `POST /v1/invocations/<id>/result`, translate to A2A Task release.

**Tests:**
- The scheduler-demo is the integration test. With `DISCOVERY_V2=true`, run setup → alice → bob and verify Bob's invocation completes.

**Commit:** "Legacy /v1 endpoints translate to A2A internally; SDK signatures unchanged"

**Risk:** Medium. Backward-compat regressions break the published SDK. Heavy regression suite.

---

## Phase D7 — mDNS publisher

**What:** Server advertises `_chakramcp._tcp` and per-agent `_a2a._tcp` records on the LAN. Default off in containers.

**New module:** `backend/relay/src/discovery/mdns.rs`

**Crate dependency:** add `mdns-sd` or equivalent. (Audit available crates first; avoid abandoned ones.)

**Behavior:**
- On startup, read `[discovery.mdns]` config from server.toml. Default-detect container env (`KUBERNETES_*`, `/proc/1/cgroup`).
- Publish `_chakramcp._tcp` instance once with TXT records.
- For each opted-in agent (`visibility=network` AND `advertise_via_mdns=true`), publish `_a2a._tcp` instance. Cap at 32; FIFO overflow (oldest registered first stays).
- Re-publish every ~90s; let TTL expire when agent's heartbeat goes stale (>5 min).
- On shutdown, withdraw records cleanly.
- Update `agents.mdns_advertised_at` on each successful publish.

**Tests:** integration tests with a test mDNS responder (or run an actual `dns-sd -B` in CI on macOS runner).

**Commit:** "mDNS publisher (Layout C): _chakramcp._tcp + per-agent _a2a._tcp"

**Risk:** Low for v1 (it's optional). Medium if we ever rely on it for production discovery.

---

## Phase D8 — CLI surface updates

**What:** New CLI behaviors for the discovery model.

**Files in `backend/cli/src/`:**

- `commands/networks.rs`: `chakramcp networks use local` — browse `_chakramcp._tcp.local` first, offer to connect to LAN-reachable server before falling back to localhost.
- `commands/agents.rs`: `chakramcp agents network` — accept `--include-mdns` (default true on `local`), show `⊕` badge for non-registered LAN peers.
- `commands/agents.rs`: `chakramcp agents create` — accept optional `--agent-card-url`. Without it, registers in pull mode.

**Tests:** CLI integration tests against a local relay running with `DISCOVERY_V2=true`.

**Commit:** "CLI: mDNS-aware networks + push-mode agent registration"

**Risk:** Low.

---

## Phase D9 — Slug allocation: tombstones + renames + reserved words

**What:** Validation rules and lifecycle for slugs.

**Files:**

- `backend/relay/src/handlers/agents.rs`: registration validation — charset, length, reserved words, NFKC normalize, partial-unique-index handling.
- `backend/relay/src/handlers/agents.rs`: `DELETE /v1/agents/<id>` → set `tombstoned_at`, do NOT free the slug.
- `backend/relay/src/handlers/agents.rs`: `PATCH /v1/agents/<id>` slug change → write `slug_aliases` row with 90d expiry, update agent slug, alias-resolution middleware redirects 301.
- `backend/relay/src/middleware/slug_redirect.rs`: new middleware that intercepts `/agents/<account>/<slug>/...` and resolves aliases to the head of any rename chain, returning 301.
- Same flow for account-level renames (180d window).
- Forced-takedown command: `chakramcp admin takedown <account>/<slug>` (CLI), backed by `forced_takedown_at` timestamp + audit log.

**Tests:**
- Unit: validation cases (reserved words, charset, length, NFKC).
- Integration: rename → 301 chain → tombstone after expiry → 410.

**Commit:** "Slug allocation: validation, tombstones, renames, redirects"

**Risk:** Medium. Slug-redirect middleware sits in the request path; bugs here affect every agent URL.

---

## Phase D10a — Discovery search infrastructure (perf-first)

**What:** The query layer for discovery, designed for speed under combined filters at 100k+ agents. Frontend lands in D10b; this phase is API + indexes + pagination only.

### Pagination contract

**Cursor-based, not offset-based.** Offset-based pagination is O(N) at the database level for deep pages and produces inconsistent results under concurrent writes. Cursor-based gives predictable performance regardless of position.

- Default page size: 20. Max: 100. Configurable per request (`limit` query param).
- Cursor: opaque base64-encoded JSON `{rank_score, agent_id}` (rank_score depends on the active sort; agent_id is the stable tiebreaker UUID).
- Response envelope:

  ```json
  {
    "agents": [...],
    "next_cursor": "eyJyYW5rIjowLjk1LCJpZCI6IjAxOWRkMC..." | null,
    "total_estimate": 1247    // present only on first page; HLL-based for unauthed, exact for authed (cheap given filters)
  }
  ```

- Stable secondary sort key (agent UUID) on every query so concurrent writes don't cause cursor drift.

### Filter strategy + indexing

| Filter | Source data | Query strategy | Index |
|---|---|---|---|
| `q` (free text) | display_name, description, account display_name, capability descriptions | Postgres FTS (`tsvector` column, refreshed via trigger on row updates) | GIN on `tsvector` — added in this phase, NOT in D1 |
| `capability_schema` | capabilities.{output,input}_schema | JSONB containment (`@>`) | `idx_capabilities_*_schema_jsonb` (already in D1) |
| `tags` | agents.tags | GIN array containment | `idx_agents_tags_gin` (already in D1) |
| `friendship` | friendships table joined via account_id | Inner join with caller's account, filter `status = accepted` | composite (caller_account_id, target_account_id) index on friendships (already exists pre-A2A) |
| `mode` | agents.mode | `WHERE mode = ?` | btree on `(mode, tombstoned_at)` partial index — added in this phase |
| `verified` | accounts.verified_at IS NOT NULL | `WHERE accounts.verified_at IS NOT NULL` | partial btree on `accounts(id) WHERE verified_at IS NOT NULL` — added in this phase |
| `include_dormant` | agents.last_polled_at / last_card_fetched_at | filter at handler level after main query | reuses health-state computation (D11) |

**Trigger-maintained `tsvector` column on agents:**

```sql
ALTER TABLE agents ADD COLUMN search_vec tsvector;
CREATE TRIGGER agents_search_vec_update BEFORE INSERT OR UPDATE ...
CREATE INDEX idx_agents_search_vec ON agents USING GIN (search_vec);
```

Updated when `display_name`, `description`, `tags`, or any owned capability description changes. The trigger keeps it consistent without a refresh job.

### Pre-computed denormalization

To make recents/trending/friends-of cheap, two counter-cache columns updated by background job (not in transaction; eventual consistency is fine):

```sql
ALTER TABLE agents ADD COLUMN friend_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN recent_invocations_7d INTEGER NOT NULL DEFAULT 0;
```

Refreshed once per minute by a tokio-scheduled job that runs cheap aggregate queries against `friendships` and `relay_log`. Stale by up to 60s — acceptable.

### Query timeout + result envelope on timeout

- Server-side query timeout: 1000ms default, 2000ms hard cap (configurable in server.toml).
- On timeout, return partial results with HTTP 200 plus header `X-ChakraMCP-Search-Truncated: true`. Frontend shows a "results truncated; refine your query" notice. Avoids blank-page-on-slow-query.

### Caching

- Public discovery responses (no caller auth, no `friendship` filter) cacheable at edge with `Cache-Control: public, max-age=60, stale-while-revalidate=300`.
- Authenticated responses per-user, app-layer Redis cache with 30s TTL, invalidated on friendship.* / grant.* / agent.tombstone events.
- Cache key includes the full normalized filter set + caller identity.

### Recents endpoint

Cheap query against `relay_log` for caller's last-N distinct target agents in the past 7 days, ordered by `MAX(created_at)`. Cached per-user 30s. Index on `relay_log(requester_account_id, created_at DESC)` already in migration doc.

### Trending endpoint

Pre-computed materialized view refreshed every 5 minutes:

```sql
CREATE MATERIALIZED VIEW trending_agents AS
  SELECT a.id, a.account_id, a.slug,
         COALESCE(recent_invocations_7d, 0) * 2 + COALESCE(friend_count, 0) AS score
    FROM agents a
   WHERE a.tombstoned_at IS NULL
     AND a.visibility = 'network'
     AND a.noindex = false
   ORDER BY score DESC
   LIMIT 200;
CREATE UNIQUE INDEX ON trending_agents (id);
```

Refresh via `REFRESH MATERIALIZED VIEW CONCURRENTLY`. View is small and stays in shared buffers.

### Rate limits

- Unauthed `GET /v1/discovery/agents`: 30 req/IP/min.
- Authed: 120 req/account/min.
- Returns 429 with `Retry-After` header when exceeded.

### Backend (Rust)

- `backend/relay/src/handlers/discovery.rs` — handlers for the five endpoints
- `backend/relay/src/discovery/query_builder.rs` — composes Postgres queries from filter sets, defends against bad filter combos
- `backend/relay/src/discovery/cursor.rs` — opaque cursor encode/decode with HMAC against tampering
- `backend/relay/src/discovery/cache.rs` — Redis layer with invalidation on event subscriptions

**Endpoints (gated by `DISCOVERY_V2`):**
- `GET /v1/discovery/agents` — search with all filters, cursor pagination
- `GET /v1/discovery/agents/<account>/<slug>` — detail
- `GET /v1/discovery/agents/<account>/<slug>/capabilities` — capability list
- `GET /v1/discovery/recents` — caller's recents (auth required)
- `GET /v1/discovery/trending` — public

### Performance benchmarks (gating the D14 flip)

A separate test target seeds the DB with realistic fixture data and runs query benchmarks via `cargo bench`:

- 10k agents, 100k capabilities, 50k friendships:
  - p99 single-filter `q` query: < 50ms
  - p99 combined `q + capability_schema + verified=true`: < 200ms
  - p99 `recents` for active caller: < 30ms
  - p99 `trending`: < 10ms (it's a pre-aggregated view)
- 100k agents (stretch goal): p99 < 500ms across all combinations

**These benchmarks must pass before D14 flag flip.**

### Tests

- Unit: cursor encoding roundtrip; query builder for each filter combination; FTS trigger correctness on row updates
- Integration: capability-shape search returns the agents whose schemas contain the filter; pagination consistency under concurrent writes (cursor doesn't drift)
- Benchmark suite (separate from CI): the perf gates above

**Commit boundary:** indexes + tsvector trigger; query builder; cursor; cache; benchmarks. ~5 commits.

**Risk:** Medium. Postgres planner gotchas under combined GIN + btree indexes; needs `EXPLAIN ANALYZE` validation. Cursor stability under concurrent writes is a subtle correctness property.

---

## Phase D10b — Discovery frontend

**What:** The user-facing pages that consume D10a's API.

**Frontend (Next.js):**

- `frontend/src/app/(site)/agents/page.tsx` — public directory (HTML + SSR for SEO; cursor pagination via URL `?cursor=...`)
- `frontend/src/app/(site)/agents/[account]/page.tsx` — account directory page
- `frontend/src/app/(site)/agents/[account]/[slug]/page.tsx` — agent home page (OG metadata from cached card)
- `frontend/src/app/(site)/agents/index.json/route.ts` — JSON paginated index for crawlers
- `frontend/src/app/(app)/app/discovery/page.tsx` — logged-in dashboard with recents, friends', trending sections + filter sidebar

**Frontend perf details:**

- Server-side rendering with Next.js's revalidation (60s) for public directory pages → cacheable at the CDN.
- Client-side filter changes hit `/v1/discovery/agents` with debounced input (200ms), AbortController on stale requests.
- Filter sidebar: optimistic updates; clear "X results found" + "results truncated" banner when applicable.
- Capability-shape search input: a JSON editor with schema validation; example schemas pre-loaded from popular agents.

**Tests:**

- Frontend snapshot / Playwright for the directory page rendering
- Crawl test against the public directory: every linked agent home page returns 200, structured data validates

**Commit boundary:** one commit per page tier, integration commit at end. ~4 commits.

**Risk:** Low.

---

## Phase D11 — Health surfacing

**What:** State machine for healthy/stale/unreachable/dormant; surfaces in discovery responses.

**Files:**

- `backend/relay/src/health/agent_state.rs`: pure function `(mode, last_polled_at, last_card_fetched_at, fetch_failure_count) → State` matching the spec's state table.
- Discovery handlers (D10) annotate each response item with a `state` field.
- Dormant filter behavior: param ignored when no auth; honored when bearer present.

**Tests:** unit tests on each state transition boundary.

**Commit:** "Health state machine + discovery surfacing"

**Risk:** Low.

---

## Phase D12 — Error code catalog

**What:** `/.well-known/error-codes.json` is the single source of truth for stable codes + URLs.

**Files:**
- `backend/relay/src/errors/catalog.rs`: Rust enum + serializer. Build an enum like `enum AppError { AuthMissing, FriendshipRequired, ... }` with `as_code() -> &'static str` and `as_jsonrpc_code() -> i32`. Catalog endpoint serializes the enum to JSON.
- All error responses across the codebase reference the enum.

**Why now (not earlier):** preceding phases used hard-coded codes; this phase consolidates them.

**Commit:** "Error code catalog: single source of truth at /.well-known/error-codes.json"

**Risk:** Low; mostly mechanical.

---

## Phase D13 — Verified-account badge

**What:** DNS TXT verification + Google Workspace SSO verification.

**Files:**
- `backend/relay/src/verification/dns_txt.rs`: resolve `chakramcp-verify=<token>` against a domain.
- `backend/relay/src/verification/workspace_sso.rs`: hook into existing OAuth flow if Google is wired; check `hd` claim matches account-claimed domain.
- `backend/relay/src/handlers/accounts.rs`: `POST /v1/accounts/<id>/verify` endpoint.
- Frontend: settings page UI for kicking off verification.

**Manual takedown procedure:** documented in `docs/operations/trademark-takedown.md`. SLA 7 days. Email `legal@chakramcp.com`. Audit-logged decisions.

**Commit:** "Verified-account badge: DNS + Workspace SSO"

**Risk:** Low. Optional opt-in feature; no critical-path code.

---

## Phase D13b — Website + messaging update (parallel to D13/D14)

**What:** Update every public surface that describes ChakraMCP to position us as **a trust + policy layer on top of A2A** rather than a competing wire protocol. Done in parallel with D13/D14 so messaging matches shipped reality on the day of the flag flip.

### Positioning shift

| Old framing | New framing |
|---|---|
| "A relay network for AI agents — register, friend, grant, invoke, audit." | "A trust + policy layer for A2A — friendships, grants, consent, audit. Agents speak A2A; we enforce who can talk to whom." |
| "Pull-based delivery — no public webhook needed." | "A2A-compatible pull mode via the inbox bridge — pull agents look like first-class A2A endpoints to callers, no public host needed." |
| Architecture diagram: agents ↔ ChakraMCP relay (custom wire) | Architecture diagram: agents ↔ ChakraMCP relay (A2A wire) ↔ agents, with explicit "A2A" labels on the wire and a callout that MCP stays internal to each agent |

The five primitives (Agents / Capabilities / Friendships / Grants / Inbox+Invocations) stay the same — that's the IP. Only the wire layer's framing changes.

### Surfaces to update

**Frontend (all in `frontend/src/`):**

| File | Change |
|---|---|
| `app/(site)/page.tsx` (landing) | Hero tagline + value-prop section. Add an "A2A-compatible" badge. |
| `app/(site)/docs/page.tsx` | Lede paragraph. Add a card linking to a new "Why A2A?" section. |
| `app/(site)/docs/concepts/page.tsx` | Add a section "ChakraMCP and A2A: the layering" explaining the trust-layer-on-A2A model. |
| `app/(site)/docs/agents/page.tsx` (autopilot) | Re-frame as "A2A-native integration with ChakraMCP trust mediation." Update code examples to show the new path-based URLs. |
| `app/(site)/docs/quickstart/page.tsx` | Update step 5 (the inbox-loop snippet) to mention A2A under the hood; SDK code example unchanged. |
| `components/sections/*.tsx` (Poster, LeadHero, RelayDiagram, CoffeeLoop labels) | Architecture diagrams show A2A as the wire. Update any text labels that describe a "ChakraMCP protocol." |
| `app/(site)/cofounder/page.tsx` | Tempo section reflects the migration as a strategic win, not a pivot. |
| `app/(site)/brand/page.tsx` | No change (visual brand only). |

**Repo-level:**

| File | Change |
|---|---|
| `README.md` | Hero paragraph + architecture ASCII art + "What ChakraMCP gives an agent" section. Add A2A reference. |
| `docs/INSTALL.md` | One-line note that the relay speaks A2A; SDK installs unchanged. |
| `frontend/public/llms.txt` | Update the project description for LLM autopilot consumption. |
| `frontend/public/.well-known/chakramcp.json` (or wherever the host descriptor lives) | Add `a2a_compatible: true`, `a2a_version: "0.3"`, `a2a_card_pattern: "/agents/<account>/<slug>/.well-known/agent-card.json"` so generic A2A clients can self-discover us. |

**SDK READMEs** (`sdks/{typescript,python}/README.md`):
- One-paragraph "What ChakraMCP does for you" update mentioning A2A.
- Code samples unchanged (signatures preserved).

### Optional new surfaces

**`docs/why-a2a.md` or `frontend/src/app/(site)/docs/a2a/page.tsx`:** a dedicated page explaining the layering — written for both humans (technical decision-makers) and LLM autopilot. Sections: "What A2A is", "What ChakraMCP adds on top", "How the relay enforces policy", "Migration history (we used to ship a custom wire; here's why we changed)."

This is the single biggest acquisition surface for A2A community traffic. Worth doing well.

### Acquisition implications

- A2A clients in the wild that hit our relay and bounce on policy now get an error message that explicitly references A2A: "this agent is policy-gated by ChakraMCP — visit chakramcp.com to friend / register." Helps the bounce-back convert.
- SEO: every agent's home page includes "A2A-compatible" + structured data so generic A2A directory crawlers find us.
- Blog post / launch announcement timed with D14: "ChakraMCP runs on A2A now" — talks to both our existing users (the SDK signature is the same; here's what changed) and potential A2A-native users (here's a trust layer you can plug in).

### Tests / verification

- Render every public page in the preview server; check that no page still says "custom protocol" or implies we're competing with A2A.
- LLM-readable surfaces (`/llms.txt`, `/.well-known/chakramcp.json`, `/docs/agents`) all consistent.
- Crawl the rendered site and grep for old phrasings; confirm zero hits before flag flip.

**Commit boundary:** one commit per surface tier (frontend, repo-level, SDK READMEs, optional new pages). 4–5 commits total.

**Risk:** Low technically. **High** for messaging — every word matters for positioning. Recommend the marketing copy gets a separate review pass (independent of the spec/plan reviewers).

---

## Phase D14 — Feature flag flip

**What:** `DISCOVERY_V2` defaults to `true`. The new model is the live one.

**Pre-flip checklist:**
- All preceding phases merged + green
- Security review of D5 (JWT key handling, JWKS rotation) signed off
- Search performance benchmark: `GET /v1/discovery/agents?capability_schema=...` p99 < 200ms with 10k agents
- scheduler-demo passes end-to-end against staging
- Migration applied successfully on staging DB
- Rollback procedure documented (env var flip + cards stop being signed; SDKs continue using legacy `/v1/...` which keeps working)

**Files:**
- `backend/relay/src/config.rs`: change default
- `frontend/src/...`: enable new discovery routes (already gated by feature flag from D10)
- `CHANGELOG.md` entry

**Commit:** "Default-on: DISCOVERY_V2 is now the live discovery model"

**Risk:** **High.** This is the cutover moment. Run during low-traffic window; have rollback ready.

---

## Phase D15 — Tear-down of pre-A2A code

**What:** Remove dead code paths. Schedule for ~30 days after D14 to give external operators (anyone running `chakramcp-server`) time to migrate.

**Files:**
- Drop `capability_runs` table (migration). All data already in `relay_log`.
- Remove old non-translating `/v1/invoke` direct-pass-through paths if any survived.
- Migration doc gets a final "this is now historical" header.

**Risk:** Low if D14 was clean.

---

## Cross-cutting work threads

These touch multiple phases:

### Observability
- Prometheus metrics: per-card-fetch latency + outcome, JWT mints/sec, parked-call count, decision-tree branch counts.
- Wire into existing metrics module incrementally (D2 cards, D5 forwarder, D9 slug redirects, D10 discovery).

### Security review checkpoints
- D2: signing key generation + storage
- D5: JWT minting + JWKS rotation
- D9: slug-redirect middleware (XSS / cache-poisoning surfaces)
- Pre-D14: full audit pass

### Documentation
- D2: agent-author guide for verifying our JWTs (`docs/agent-authors/auth-with-chakramcp.md`)
- D8: CLI help text + man pages
- D10: API reference (regenerate from handler signatures)
- D14: changelog + blog post

### Test infrastructure
- D2 onward: each phase ships with its own integration test in `backend/relay/tests/`
- D6: scheduler-demo's smoke run becomes a CI job that exercises the full pipeline

---

## Estimated phase weight

Rough ordering by effort, not committing to dates:

| Phase | Weight | Notes |
|---|---|---|
| D0 | XS | migration-doc patch |
| D1 | S | schema + indexes |
| D2 | L | agent-card service, security-sensitive |
| D3 | XS | A2A endpoint stubs |
| D4 | M | policy decision tree + tests |
| D5 | XL | core proxy + JWT + inbox bridge |
| D6 | M | SDK translation shims; demo regression |
| D7 | M | mDNS publisher |
| D8 | S | CLI updates |
| D9 | M | slug lifecycle middleware |
| **D10a** | **L** | **search infrastructure (perf-first, benchmarks gate D14)** |
| D10b | M | discovery frontend pages |
| D11 | S | health state machine |
| D12 | S | error catalog |
| D13 | S | verified-account badge |
| D13b | M | website + messaging update (parallel to D13/D14) |
| D14 | M | flag flip + monitoring + launch announcement |
| D15 | S | tear-down of pre-A2A code |

D5 and D10a are the two heaviest. Plan accordingly. The D10a benchmarks (p99 < 200ms at 10k agents) are a pre-D14 gate, not a nice-to-have.

---

## What's deliberately NOT in this plan

- Cross-network friendship import (v2)
- Reputation signals (v2)
- Compliance attestation filters (v2)
- Geographic / latency-aware routing (v3+)
- Vanity flat slugs (v2)
- Full JSON Schema validation in capability search (v2)
- Self-serve trademark takedown form (v2)
- Per-grant or per-capability parking timeout overrides (additive when needed)

Each is in the discovery spec's "Open questions / explicitly deferred" table. Don't pull them into this scope.

---

## Sign-off needed

Before any work starts:

1. Approval of this phase ordering.
2. Confirmation of the feature flag name (`DISCOVERY_V2`) and the default-off → default-on path.
3. Confirmation that the existing scheduler-demo as the regression test is acceptable (vs writing a new dedicated A2A integration test).
4. Confirmation that D5's security review is on the critical path (recommended: yes; please don't skip).
