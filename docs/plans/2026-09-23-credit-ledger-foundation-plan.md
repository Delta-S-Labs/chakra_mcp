# Credit ledger foundation — implementation plan (Phase 1)

_Plan for [docs/specs/2026-09-23-credit-ledger-foundation-design.md](../specs/2026-09-23-credit-ledger-foundation-design.md).
Drafted 2026-09-23; rebuilt 2026-09-25 (charge queue, review fixes, prerequisites)._

**Governing rule:** never hamper invocation performance. The invocation path reads
in-memory switches; everything else runs in parallel.

## Order

| # | PR | Deploy | Depends on |
|----|----|----|----|
| 1 | **PR-A** — usage middleware off the hot path | yes | — |
| 2 | **PR-B** — reject legacy `/v1/invoke` to push-mode agents | yes | — |
| 3 | **PR1** — expand migration `0033` | yes (migration) | prod guard query = 0 |
| 4 | **PR2** — swap to async credits | yes (binary only) | PR1 live |
| 5 | **PR3** — contract migration `0034` | yes (migration, must succeed) | PR2 live |

⚠️ **One merge at a time.** CD's concurrency group drops a middle deploy on a burst.
After each merge, confirm the commit's CD run exists and `Restart relay` + `Probe
public health` are green (`gh run list --commit <sha>`, `gh run view <id>`).

⚠️ **Both binaries run migrations at boot** (`relay/src/main.rs:29`,
`server/src/main.rs:175`) and refuse to start if the DB has a migration they lack.
Never revert a migration file; revert code.

---

## PR-A — Usage middleware off the hot path

**Problem.** Every relay REST request (A2A, `/v1/invoke`, …) re-authenticates the
caller in `usage_middleware` *before* the handler (a DB query: revocation check or
API-key lookup) and awaits a `usage_events` INSERT *before* returning. Every MCP tool
call awaits the same INSERT (`mcp.rs:576`).

**Design.**
- `events::UsageRecorder` — `Noop` or `Channel` (bounded `tokio::sync::mpsc`,
  capacity 10,000). `record(event)` is a **sync `try_send`**: it never waits. When the
  channel is full the event is dropped and counted (usage analytics are best-effort,
  as they are today).
- `UsageEvent { actor, account_id, surface, action, method, route, status_code }`
  with `actor` = `Anonymous` | `Header(String)` (raw `Authorization`, resolved later) |
  `User(AuthUser)` (already authenticated, e.g. MCP).
- A background writer drains the channel in batches (whatever is queued, up to 500),
  resolves each distinct `Header` once per batch via `auth::authenticate`, and
  inserts the batch in one transaction using the existing `record_usage` SQL
  (unchanged text → `.sqlx` unchanged; the function becomes generic over the
  executor). Errors are logged and swallowed.
- Middleware: clone the `Authorization` header string, `next.run(req).await`,
  `state.usage.record(…)`. No other awaits.
- MCP `:576`: `state.usage.record(… User(user.clone()) …)`.
- `RelayState` gains `usage: UsageRecorder` via `.with_usage_recorder()`;
  `RelayState::new` defaults to `Noop`, so test callers are unaffected. Both mains
  build the state, spawn the writer with a state clone (for auth), then attach the
  recorder.
- A `Flush(oneshot)` message lets tests wait for the writer deterministically.

**Tests.** A recorder at capacity drops (and counts) instead of blocking; `Noop`
never errors; with a live writer + flush, a REST request yields one `usage_events`
row attributed to its API key/JWT user; the MCP test at `:1953` flushes before
asserting.

**Done when:** deploy green; `usage_events` rows still appear in prod for A2A and
MCP traffic.

---

## PR-B — Reject legacy `/v1/invoke` to push-mode agents

**Problem.** `invoke.rs:44-47`: v0.1 SDKs invoking a **push-mode** target via
`/v1/invoke` get a `pending` row no one ever delivers. Under credits it would be
charged every time.

**Design.** Before dispatch, if the target is push-mode, reject with a clear error
pointing at the A2A endpoint, recorded via the existing `record_terminal` path
(`rejected` — never charged). Pull-mode targets (every v0.1 use case to date) are
unaffected.

**Tests.** Push-mode target → rejected + recorded, no `pending` row; pull-mode →
unchanged (the `legacy_v01_contract_tests` stay green).

---

## PR1 — Expand migration

**Before merging:** run against prod and confirm `0`:
`SELECT count(*) FROM accounts a JOIN plans p ON p.id = a.plan_id WHERE p.name <> 'free';`

**File:** `backend/migrations/0033_credits_expand.sql`
1. `SET LOCAL lock_timeout = '5s';`
2. Guard: `DO $$ … RAISE EXCEPTION … $$` if any account is on a non-free plan.
3. Create `credit_wallets` (+ period index), `credit_ledger` (+ indexes, purchase
   CHECK, unique purchase ref), `credit_charge_queue` (autovacuum reloptions),
   `invocation_charges` (+ account index) — exactly as in the spec.
- Touches neither `relay_invocations` nor `accounts`. No backfill.

**Tests:** the guard test — `#[sqlx::test(migrations = false)]`,
`Migrator::run_to(32)`, seed a `pro` account with runtime `sqlx::query`, assert
`run_to(33)` fails; then the clean path succeeds and all four tables exist. Runtime
queries keep `.sqlx` unchanged.

**Done when:** deploy green; the four tables exist in prod; old enforcement still works.

---

## PR2 — Swap to async credits

**Files:**
- `relay/src/limits/credits/config.rs` — `CreditsConfig` (`Default`, `from_env`,
  `validate`: every value parses and is > 0, else startup fails).
- `relay/src/limits/credits/cache.rs` — blocked set + rate overrides + `refreshed_at`;
  `is_blocked` returns `false` when the cache is older than 3 intervals.
- `relay/src/limits/credits/worker.rs` — supervised loop: charge (spec SQL, batches of
  1,000, loop within a time budget) and grant (spec SQL), each its own transaction
  that first takes `pg_try_advisory_xact_lock` as a separate statement; refresh on
  every instance; per-step errors logged; re-spawn on panic; per-tick logs.
- `relay/src/limits/mod.rs` — `enforce` with no DB: rate (Redis, cached per-account
  limit) → `is_blocked`; drop `resolve_plan`/`PlanLimits`; `QuotaExceeded` →
  `InsufficientCredits`.
- `relay/src/limits/quota.rs` — delete.
- `relay/src/state.rs` — cache + `CreditsConfig` via builders (defaults in `new`).
- `relay/src/main.rs`, `server/src/main.rs` — parse + validate config, run one
  refresh **before serving**, spawn the supervised worker.
- Enforce call sites: `handlers/a2a.rs:79`, `handlers/invoke.rs:478` and `:675`,
  `handlers/mcp.rs:936`.
- Write sites — replace `BEGIN` → INSERT → `quota::increment` → `COMMIT` with the
  single enqueueing statement: `forwarder.rs:230-259`, `inbox_bridge.rs:129-152`,
  `handlers/invoke.rs:532-556` and `:692-714`, `handlers/mcp.rs:957-980`.
- Renames: `policy/decision.rs`, `shared/src/error.rs`, `mcp.rs:156`,
  `a2a.rs:92`; tests in `a2a.rs` (`:1025` string; `:1024`'s `-32008` stays; drop the
  `usage_counters` helpers at `:962`/`:1037`), `invoke.rs` (`:2267`, `:2230`).
- Remove every `.rs` reference to `usage_counters`/`plans`/`plan_id` except the PR1
  guard test.
- `backend/.sqlx/**` — regenerate.

**Tests:** per the spec's Testing section (charge, grant, switch/refresh,
supervision/config, enforce, five sites, invariant, live Redis).

**Rollout:** optional — set `LIMITS_ENFORCE=false` before merging, watch the tick
logs, re-enable. Release note: `chk.limit.credits` / `account_credits_exhausted`
replace the quota codes.

**Done when:** deploy green; tick logs show the queue draining to ~0, wallets
created and granted as accounts invoke, and an exhausted test account is denied.

---

## PR3 — Contract migration

**File:** `backend/migrations/0034_credits_contract.sql` — drop `accounts.plan_id`,
`plans`, `usage_counters`.

**Must-succeed:** once applied, the PR2 binary can no longer boot. Merge only with PR2
live and healthy.

**Tests:** a fresh DB migrates `0001..0034` cleanly; full suite green; `.sqlx`
unchanged.

---

## Cross-cutting

- **`.sqlx`** is a required CI check and the prod build input: regenerate against a
  migrated `:5544` DB after any query change
  (`cd backend && cargo sqlx prepare --workspace -- --tests`).
- **Pre-commit hook** needs `DATABASE_URL` (`:5544`) exported when `.rs` files are staged.
- **Local deps:** Postgres `:5544` (`docker start chakra-sqlx-pg`), Redis `:6379`
  (`docker start chakra-test-redis`, `REDIS_TEST_URL`).
- **Rollback:** revert code, never migration files. PR3 is forward-only.
