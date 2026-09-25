# Credit ledger foundation — implementation plan (Phase 1)

_Plan for [docs/specs/2026-09-23-credit-ledger-foundation-design.md](../specs/2026-09-23-credit-ledger-foundation-design.md).
Drafted 2026-09-23; rebuilt 2026-09-25 for **asynchronous credit accounting**._

## Rollout strategy

Enforcement is live (`LIMITS_ENFORCE=true`) and CD migrates **before** restarting
the relay, so the change is **expand → swap → contract**, one PR and one deploy
each:

| PR | Deploy | What | Old relay during the migrate window |
|----|----|----|----|
| **PR1** | A | Expand migration `0033` (additive) + config plumbing | untouched — still on `usage_counters`/`plans` ✓ |
| **PR2** | B | Swap: async worker + in-memory gate + hot-path cleanup (no migration) | n/a — pure binary swap |
| **PR3** | C | Contract migration `0034` (drop `plan_id`/`plans`/`usage_counters`) | already the PR2 binary, which ignores them ✓ |

⚠️ **Do not burst-merge** (CD's concurrency group drops a middle deploy). Merge
PR1 → confirm its CD run exists and is green (`gh run list --commit <sha>`) →
PR2 → confirm → PR3.

---

## PR1 — Expand migration + config plumbing

**Goal:** add the credit schema and config. No behavior change: the old code never
sets `charged_account_id`, so nothing is charged yet.

**Files:**
- `backend/migrations/0033_credits_expand.sql` (new)
- `backend/shared/src/config.rs` (`SharedConfig` + `from_env` + parse test)
- `backend/server/src/main.rs` (`load_config` builds `SharedConfig` — add the new
  fields with defaults so it compiles; TOML keys are deferred to the self-host
  checklist)
- `infra/docker-compose.prod.yml` (new env with defaults)

**Steps:**
1. `0033_credits_expand.sql`, in order:
   0. Guard — refuse to run if any account is on a non-free plan:
      ```sql
      DO $$ BEGIN
        IF EXISTS (SELECT 1 FROM accounts a JOIN plans p ON p.id = a.plan_id
                    WHERE p.name <> 'free') THEN
          RAISE EXCEPTION 'credits migration: accounts on non-free plans exist; map them first';
        END IF;
      END $$;
      ```
   1. `CREATE TABLE credit_wallets (…)` + `credit_wallets_grant_period_idx` (schema per spec).
   2. `CREATE TABLE credit_ledger (…)` with `id … DEFAULT gen_random_uuid()` +
      `credit_ledger_account_created_idx` + the unique partial index
      `credit_ledger_purchase_ref_uniq ON (external_ref) WHERE reason = 'purchase'`.
   3. `ALTER TABLE relay_invocations ADD COLUMN charged_account_id UUID, ADD COLUMN credits_charged_mc BIGINT;`
      then `CREATE INDEX relay_invocations_uncharged_idx ON relay_invocations (created_at) WHERE charged_account_id IS NOT NULL AND credits_charged_mc IS NULL;`
      (plain build is fine at current size; note `CONCURRENTLY` + a `-- no-transaction`
      migration for large deployments).
   4. `INSERT INTO credit_wallets (account_id) SELECT id FROM accounts;`
2. `SharedConfig`: add `hosting_mode: HostingMode` (`Managed`|`SelfHosted`, default
   `SelfHosted`, case-insensitive), `credits_default_monthly_free_mc: i64` (100000),
   `credits_cost_per_invocation_mc: i64` (100), `credits_sweep_interval_secs: u64` (5),
   `limits_default_rate_per_min: i32` (60). Numerics via `.parse().unwrap_or(default)`.
   Update every construction site (`from_env`, the struct literal + its test at
   `config.rs:105-114`, and the server's `load_config`). `LIMITS_ENFORCE` stays
   read in main via `enforce_flag`.
3. Compose: add the five env with their defaults
   (`HOSTING_MODE: ${HOSTING_MODE:-self_hosted}` etc.).

**Tests:** `#[sqlx::test]` — the guard raises when a non-free account exists;
after `0033`, every account has a wallet (balance 0, period NULL), the new columns
and indexes exist, and a duplicate `purchase` `external_ref` is rejected. Config
parse: defaults when unset, explicit values, `HOSTING_MODE` casing.

**CI/deploy:** `.rs` + migration → full backend CI + CD. `.sqlx` unchanged (no new
`query!` macros). **Done when:** deploy A green; tables present in prod;
`SELECT count(*) FROM credit_wallets` = accounts; old enforcement still working.

---

## PR2 — Swap to async credits

**Goal:** remove all credit/quota DB work from the invocation path; charge, grant
and block via the background worker.

**Files:**
- `backend/relay/src/limits/credits/mod.rs` (new) — `CreditCache` (blocked set +
  rate overrides; `is_blocked`, `rate_limit_for`, `replace`), shared via `Arc`.
- `backend/relay/src/limits/credits/worker.rs` (new) — tick loop, charge, grant,
  refresh, `spawn_worker(db, cache, cfg)`.
- `backend/relay/src/limits/mod.rs` — `enforce` without DB: rate check (limit from
  cache or default) → `cache.is_blocked`; remove `resolve_plan`/`PlanLimits`/`check`'s
  DB calls; `LimitOutcome::QuotaExceeded` → `InsufficientCredits`.
- `backend/relay/src/limits/quota.rs` — delete.
- `backend/relay/src/state.rs` — `RelayState` gains `credit_cache: Arc<CreditCache>`
  via a builder (`.with_credit_cache(…)`); `RelayState::new` defaults to an empty
  cache, so its ~84 test callers stay untouched.
- `backend/relay/src/main.rs`, `backend/server/src/main.rs` — build one cache,
  attach it to state, `spawn_worker`.
- **Enforce call sites** (new signature): `handlers/a2a.rs:79`,
  `handlers/invoke.rs:478`, `handlers/invoke.rs:675`, `handlers/mcp.rs:936`.
- **Write sites** — add `charged_account_id` to the INSERT; delete
  `quota::increment` and the transaction wrapper where it existed only for it
  (verify each; keep the txn if it wraps other statements):
  `forwarder.rs::persist_invocation` (~`:258`), `inbox_bridge.rs::park` (~`:151`),
  `handlers/invoke.rs` `invoke_trusted` (~`:555`) / `invoke_public` (~`:713`),
  `handlers/mcp.rs::invoke` (~`:979`).
- `policy/decision.rs` (`DenyReason` rename), `shared/src/error.rs` (`ApiError`
  rename), `handlers/mcp.rs:156` (limit-error arm), `handlers/a2a.rs:92`
  (`LimitOutcome`→`DenyReason` mapping).
- Tests: `a2a.rs` (`chk.limit.quota`→`chk.limit.credits` at `:1025` — `:1024`'s
  `-32008` **stays**; remove `usage_counters` helpers `:962`/`:1037`), `invoke.rs`
  (`account_monthly_quota_exhausted`→`account_credits_exhausted` `:2267`, test name
  `:2230`), `limits/mod.rs` tests.
- `backend/.sqlx/**` (regenerate).

**Steps:**
1. `CreditCache` + unit tests.
2. Worker — each tick:
   - **Charge** in batches of ~1,000: each batch is its **own short transaction**
     that first takes `pg_try_advisory_xact_lock(<fixed key>)` (skip if not
     acquired), then runs the spec's single statement. Loop while a full batch was
     processed, within a time budget. Never hold row locks across batches — a
     pull-mode claim on a row being stamped must wait at most one batch statement.
   - **Grant** in its own short transaction, same lock: set-based `UPDATE … RETURNING`
     → ledger `INSERT`; month difference as integer arithmetic on first-of-month
     dates; cap 12. Writers set `credit_wallets.updated_at` explicitly (no trigger).
   - Every instance: **refresh** the cache (blocked:
     `NOT unlimited AND free_grant_period IS NOT NULL AND balance_mc < cost`; rate
     overrides: non-NULL `rate_limit_per_min`).
   - Log per tick: rows charged, accounts touched, grants applied, blocked-set
     size, backlog remaining, duration; warn when the backlog grows across ticks.
3. `enforce`: rate check → blocked check; shadow logic unchanged (`limit.would_block`).
4. Write sites: bind `charged_account_id` per the spec's table
   (`authz.caller_account_id` / `row.grantee_account_id` / `invoker.account_id`).
5. Error renames per the spec (A2A keeps `-32008`; `/v1/invoke` 429; MCP JSON-RPC
   `ERR_INVALID_REQUEST`).
6. **Remove every `.rs` reference to `usage_counters`/`plans`/`plan_id`**, including
   test helpers — a hard PR2 deliverable so PR3's `cargo sqlx prepare` succeeds.
   Grep to confirm zero non-migration references.
7. `cd backend && cargo sqlx prepare --workspace -- --tests` → commit `.sqlx`.

**Tests** (see the spec's Testing section): charge (stamping, balance decrement,
history untouched, idempotent re-run, no double-charge under concurrent sweeps,
new-account wallet creation, deleted-account ids skipped); grant (NULL period,
rollover, cap, idempotent within a month, unlimited still granted); refresh
(blocked/not-blocked cases incl. never-granted and unlimited); enforce (deny /
shadow allow / rate override); five write sites (correct `charged_account_id`, no
synchronous credit write); exhausted-account denial per surface — tests populate
the cache via a real refresh — A2A + `/v1/invoke` → 429, MCP → JSON-RPC error;
the ledger invariant; live-Redis rate path green.

**CI/deploy:** full backend CI. **Rollout:** optionally set `LIMITS_ENFORCE=false`
before merging, watch the first ticks, re-enable. First tick grants every wallet
one month. **Done when:** deploy B green; tick logs show grants for all wallets,
then charges flowing with a near-zero backlog; new `relay_invocations` rows carry
`charged_account_id` and get `credits_charged_mc` within seconds; an exhausted test
account is denied.

---

## PR3 — Contract migration

**Goal:** drop the dead quota machinery.

**Files:** `backend/migrations/0034_credits_contract.sql` (new).

**Steps:**
1. `ALTER TABLE accounts DROP COLUMN plan_id;` (its FK goes with it), then
   `DROP TABLE plans;` `DROP TABLE usage_counters;`
2. Confirm zero non-migration references remain (PR2 removed them).
3. `cargo sqlx prepare` — expect no change; commit if it changes.

**Tests:** a fresh DB migrates `0001..0034` cleanly; the full backend suite passes.

**CI/deploy:** merge only after PR2 is live. **Done when:** deploy C green; the three
objects are gone from prod.

---

## Cross-cutting

- **`.sqlx`** is load-bearing (required CI check + prod `SQLX_OFFLINE` build);
  regenerate against a migrated `:5544` DB after any query change.
- **Pre-commit hook** needs `DATABASE_URL` (`:5544`) exported when `.rs` files are staged.
- **Local deps:** Postgres `:5544` (`docker start chakra-sqlx-pg`), Redis `:6379`
  (`docker start chakra-test-redis`) with `REDIS_TEST_URL` set.
- **Rollback:** PR1/PR2 revert cleanly (PR1's tables are inert without PR2; PR2
  reverted brings back the old sync quota path, which still works because PR3 hasn't
  run). PR3 is forward-only → restore from backup.

## Verification after each deploy

Confirm the per-commit CD run exists with `Restart relay` + `Probe public health`
green (`gh run view <id>`), then spot-check the DB and the worker's tick logs
(PR1: tables + wallets; PR2: grants, charges, backlog; PR3: objects gone).

## Out of scope

Owner visibility (P2), admin credit management incl. manual corrections (P3),
Dodo purchasing — managed-only (P4), request-increase + alerts (P5), agent signaling
(P6), refund automation, free/paid buckets, free-balance cap, and the
[first-self-hoster checklist](../specs/2026-09-23-credit-ledger-foundation-design.md#first-self-hoster-checklist-deferred).
