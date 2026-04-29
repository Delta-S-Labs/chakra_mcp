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

## Phase D10 — Discovery API + frontend

**What:** The user-facing discovery experience.

**Backend (Rust):**

- `backend/relay/src/handlers/discovery.rs`:
  - `GET /v1/discovery/agents` with all filters (q, capability_schema, tags, friendship, mode, verified, include_dormant)
  - `GET /v1/discovery/agents/<account>/<slug>`
  - `GET /v1/discovery/agents/<account>/<slug>/capabilities`
  - `GET /v1/discovery/recents` (auth required)
  - `GET /v1/discovery/trending` (public)

- JSONB containment query for `capability_schema` filter using existing GIN index.
- Trust status fields populated when caller bearer is present.

**Frontend (Next.js):**

- `frontend/src/app/(site)/agents/page.tsx` — public directory (HTML + SSR for SEO)
- `frontend/src/app/(site)/agents/[account]/page.tsx` — account directory page
- `frontend/src/app/(site)/agents/[account]/[slug]/page.tsx` — agent home page (with OG metadata from cached card)
- `frontend/src/app/(site)/agents/index.json/route.ts` — JSON paginated index for crawlers
- `frontend/src/app/(app)/app/discovery/page.tsx` — logged-in discovery dashboard with recents, friends', trending sections + filters

**Tests:**
- Backend unit tests on each filter
- Backend integration test for capability-shape search returning expected agents
- Frontend snapshot/Playwright for the directory page rendering

**Commit boundary:** backend handlers in one commit; frontend pages in another; integration commit ties them in.

**Risk:** Medium. Search performance under realistic data needs profiling before D14.

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
| D0 | XS | doc-only |
| D1 | S | schema + indexes |
| D2 | L | foundation, security-sensitive |
| D3 | XS | stubs |
| D4 | M | policy logic + tests |
| D5 | XL | core proxy + JWT + bridge |
| D6 | M | translation shims; demo regression |
| D7 | M | mDNS new territory |
| D8 | S | CLI updates |
| D9 | M | slug lifecycle middleware |
| D10 | L | API + frontend |
| D11 | S | state machine |
| D12 | S | catalog consolidation |
| D13 | S | verification flows |
| D14 | M | flip + monitoring |
| D15 | S | cleanup |

D5 is by far the biggest. Plan accordingly.

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
