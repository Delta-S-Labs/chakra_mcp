# Credit ledger foundation — implementation plan (Phase 1)

_Plan for [docs/specs/2026-09-23-credit-ledger-foundation-design.md](../specs/2026-09-23-credit-ledger-foundation-design.md). Drafted 2026-09-23._

## Rollout strategy

Enforcement is **live** (`LIMITS_ENFORCE=true`) and CD runs migrations **before**
restarting the relay, so the schema change is **expand → swap → contract** across
three PRs / three deploys. Each step is safe on its own:

| PR | Deploy | What | Old relay during migrate window |
|----|----|----|----|
| **PR1** | A | Expand migration `0033` (additive) + config plumbing | still on `usage_counters`/`plans` — both still exist ✓ |
| **PR2** | B | Swap enforcement to credits (module + 5 consume sites) | n/a (new relay); nothing reads `usage_counters`/`plans` after this |
| **PR3** | C | Contract migration `0034` (drop `plan_id`/`plans`/`usage_counters`) | new relay already ignores them ✓ |

⚠️ **CD concurrency:** these are three `backend/**` merges — do **not** land them in
a burst (see the `cd-concurrency-drops-intermediate-deploys` gotcha). Merge PR1,
confirm its CD run + deploy, then PR2, confirm, then PR3. Verify each with
`gh run list --commit <sha>`.

---

## PR1 — Expand migration + config plumbing

**Goal:** add the credit substrate schema (additive, non-breaking) and the config
it will use. No behavior change; the live relay keeps enforcing via the old quota
path (both old and new tables coexist).

**Files:**
- `backend/migrations/0033_credits_expand.sql` (new)
- `backend/shared/src/config.rs` (`SharedConfig` + env parsing)
- `infra/docker-compose.prod.yml` (new env with defaults)

**Steps:**
1. Migration `0033_credits_expand.sql`, in this order (INSERT must follow CREATE):
   1. `ALTER TABLE accounts ADD COLUMN credit_balance_mc BIGINT NOT NULL DEFAULT 0, ADD COLUMN monthly_free_grant_mc BIGINT, ADD COLUMN rate_limit_per_min INTEGER, ADD COLUMN free_grant_period DATE;`
   2. `CREATE TABLE credit_ledger (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), …)` + `CREATE INDEX credit_ledger_account_created_idx ON credit_ledger (account_id, created_at DESC);` (schema per spec). **The `id` DEFAULT is load-bearing** — it lets both the seed below and the runtime `settle_free_grant` INSERT (PR2) omit `id` without a NOT-NULL PK violation. `gen_random_uuid()` is available (`pgcrypto` created in `0001_users_and_orgs.sql:6`). (This deviates from the app-generated uuid-v7 convention used for other PKs; fine for a ledger where `created_at` carries ordering.)
   3. Backfill rate limit: `UPDATE accounts a SET rate_limit_per_min = p.rate_limit_per_min FROM plans p WHERE p.id = a.plan_id;`
   4. Seed balances + ledger + marker (single statement so they can't diverge; `id` via the table default):
      ```sql
      WITH seeded AS (
        UPDATE accounts
           SET credit_balance_mc = 100000,          -- MUST match CREDITS_DEFAULT_MONTHLY_FREE_MC
               free_grant_period  = date_trunc('month', now() AT TIME ZONE 'UTC')::date
         RETURNING id, credit_balance_mc
      )
      INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc)
      SELECT id, 100000, 'migration_seed', credit_balance_mc FROM seeded;
      ```
     Note: this touches every row (no `WHERE`), so the `accounts_updated_at` trigger
     (`0001:130-132`) bumps every `updated_at`. Harmless at current scale (all on
     `free`), but flag if anything keys off `updated_at`. It runs while the old
     relay still serves — additive, so safe.
2. `SharedConfig`: add `hosting_mode: HostingMode` (`Managed`|`SelfHosted`, default
   `SelfHosted`), `credits_default_monthly_free_mc: i64` (100000),
   `credits_cost_per_invocation_mc: i64` (100), `limits_default_rate_per_min: i32`
   (60). Parse from `HOSTING_MODE` / `CREDITS_DEFAULT_MONTHLY_FREE_MC` /
   `CREDITS_COST_PER_INVOCATION_MC` / `LIMITS_DEFAULT_RATE_PER_MIN`. Update the
   `from_env` struct literal + its parse test (`config.rs:105-114`); numerics via
   `.parse().unwrap_or(default)`, `HOSTING_MODE` case-insensitive defaulting to
   `SelfHosted` (matches compose `${HOSTING_MODE:-self_hosted}`). Keep the literal
   `100000` in the migration and the config default in sync (comment both). Note
   `LIMITS_ENFORCE` stays read directly in main via `enforce_flag` (not
   `SharedConfig`) — leave it as is.
3. `docker-compose.prod.yml`: add the four env with the defaults above (so a fresh
   deploy is explicit); `HOSTING_MODE: ${HOSTING_MODE:-self_hosted}`.

**Tests:**
- `#[sqlx::test]` migration smoke: after `0033`, an account has `credit_balance_mc
  = 100000`, `free_grant_period = this month`, a `migration_seed` ledger row, and
  `rate_limit_per_min` = its plan's value.
- Config parse test: env → fields; defaults when unset; `HOSTING_MODE` casing.

**CI/deploy:** `.rs` + migration → full backend CI + CD. `.sqlx` unchanged (no new
`query!` macros — the seed is raw migration SQL). **Done when:** deploy A green,
`credit_ledger` + columns present in prod, old enforcement still working.

---

## PR2 — Swap enforcement to credits

**Goal:** replace the monthly-quota check/increment with credit balance
check/consume across all four surfaces, via the shared `limits` primitive. This is
the atomic "switch to credits" change.

**Files:**
- `backend/relay/src/limits/credits.rs` (new; replaces `quota.rs`)
- `backend/relay/src/limits/quota.rs` (delete)
- `backend/relay/src/limits/mod.rs` (`resolve_limits`/`AccountLimits`, `check`, `enforce`, `LimitOutcome`)
- `backend/relay/src/limits/rate.rs` (unchanged)
- `backend/relay/src/policy/decision.rs` (`DenyReason::QuotaExceeded` → `InsufficientCredits`)
- `backend/shared/src/error.rs` (`ApiError::QuotaExceeded` → `InsufficientCredits`)
- `backend/relay/src/handlers/a2a.rs` — the A2A enforce call (`:79`, thread cfg),
  the `LimitOutcome::QuotaExceeded => DenyReason::QuotaExceeded` mapping (`:92`,
  rename to the credit variant), and test updates: `chk.limit.quota`→`chk.limit.credits`
  (`:1025`; note `:1024` asserts `-32008`, which **stays**), and remove the
  `usage_counters` test helpers (`:962`/`:1037`).
- `backend/relay/src/state.rs` — **no new builder needed**: `RelayState` already
  carries `Arc<SharedConfig>` (currently `#[allow(dead_code)]`, `state.rs:12-13`);
  read `state.config.credits_*` directly.
- `backend/relay/src/main.rs`, `backend/server/src/main.rs` (already build `RelayState` with the config; verify nothing else needed)
- consume sites: `forwarder.rs::persist_invocation`, `inbox_bridge.rs::park`,
  `handlers/invoke.rs` (`invoke_trusted`, `invoke_public`; also rename test strings
  `account_monthly_quota_exhausted`→`account_credits_exhausted` `:2267` + test name `:2230`),
  `handlers/mcp.rs::invoke`
- `backend/.sqlx/**` (regenerate)

**Steps:**
1. `credits.rs`:
   - `resolve_monthly_free_grant_mc` / `resolve_rate_limit` — `COALESCE(account col, config default)`.
   - `settle_free_grant(tx, account, default_grant_mc)` — the CTE from the spec:
     guarded `UPDATE ... WHERE free_grant_period IS NOT DISTINCT FROM observed`
     with `RETURNING`, gating the `free_grant` ledger INSERT (which **omits `id`**,
     relying on the table default from PR1); 12-month catch-up cap. `month_diff` is
     computed in **Rust** (not a PG builtin) from the observed period — unit-test the
     off-by-one: brand-new (`NULL` → 1 month) and dormant (cap 12).
   - `balance(db, account) -> i64`.
   - `consume(tx, account, cost_mc)` — `UPDATE accounts SET credit_balance_mc =
     credit_balance_mc - cost_mc WHERE id = $acct` (unconditional; best-effort per spec).
2. `mod.rs`:
   - `PlanLimits`→`AccountLimits { rate_limit_per_min, monthly_free_grant_mc }`;
     `resolve_plan`→`resolve_limits` (read account columns, no `plans` join).
   - `check(db, limiter, cfg, account)`: rate check (unchanged) → `settle_free_grant`
     → `balance >= cost_per_invocation_mc` else `InsufficientCredits`.
   - `LimitOutcome::QuotaExceeded` → `InsufficientCredits`; `enforce` shadow logic
     unchanged (`limit.would_block`).
3. Error codes: `DenyReason::InsufficientCredits` keeps jsonrpc `-32008`, data code
   `chk.limit.credits`, msg "insufficient credits"; `ApiError::InsufficientCredits`
   → 429 `account_credits_exhausted`. Update the `jsonrpc_to_http` / ApiError match
   arms (careful: the real `jsonrpc_to_http`, per the prior -32008 mapping bug).
4. Consume sites: replace each `quota::increment(&mut *tx, account)` with
   `credits::consume(&mut *tx, account, cfg.cost_per_invocation_mc)` in the same
   row-write txn. Delete the account-quota SELECT/INSERT `usage_counters` usage.
5. `state.rs` + mains: read `state.config.credits_*` directly (RelayState already
   holds `Arc<SharedConfig>`, currently `dead_code`) — no new builder.
6. **Remove every `.rs` reference to `usage_counters`/`plans`/`plan_id`, including
   the `a2a.rs`/`invoke.rs` test helpers** — a hard PR2 deliverable so PR3's
   `cargo sqlx prepare` against the post-`0034` DB (and the required `.sqlx` CI
   check) succeeds. Grep to confirm zero non-migration references remain.
7. `cargo sqlx prepare --workspace -- --tests` (new credit queries) → commit `.sqlx`.

**Tests (port + extend the existing quota tests):**
- `credits`: consume decrements by `cost_mc`; insufficient → `InsufficientCredits`;
  `settle_free_grant` applies once/month, rolls over, caps at 12, idempotent under a
  simulated concurrent settle (no double grant / no spurious ledger row);
  `resolve_limits` with and without per-account overrides; config-default fallback.
- `enforce`: shadow (`would_block`, allow) vs enforce (deny) with credits.
- Four surfaces: port `successful_pull_increments_quota_counter` et al. to assert
  `credit_balance_mc` dropped by `cost_mc`; exhausted-account denial per surface —
  A2A push/pull + `/v1/invoke` trusted+public → **429** (`account_credits_exhausted`);
  **MCP → JSON-RPC `ERR_INVALID_REQUEST`** with the "insufficient credits" message
  (mcp.rs maps limit errors to a client-side back-off error, **not** 429).
- Live-Redis rate path still green (reuse `backend-ci` Redis).

**CI/deploy:** full backend CI (incl. the ported 4-surface tests). **Rollout:**
optional safety bake — after deploy B, flip `LIMITS_ENFORCE=false` briefly, confirm
credit accounting via `limit.would_block` + balances, then re-enable. Behavior
should be equivalent (seed 100k mc = 1000 invocations = prior free quota). ⚠️ Note
shadow mode is **side-effectful on the credit tables**: `settle_free_grant` (grants
+ ledger rows) and `consume` (balance draw-down) run in the row-write txn
regardless of `LIMITS_ENFORCE` — shadow only suppresses the *deny*. So the bake
still moves real balances; that's intended (grants should always apply; consumption
draws down as it does today), just be aware.
**Done when:** deploy B green; a test account's balance visibly draws down per
invocation in prod logs/DB; over-balance → 429.

---

## PR3 — Contract migration

**Goal:** drop the now-dead quota machinery.

**Files:** `backend/migrations/0034_credits_contract.sql` (new); delete any residual
`plans`/`usage_counters` references in test fixtures.

**Steps:**
1. `ALTER TABLE accounts DROP COLUMN plan_id;` (drop FK first if named), then
   `DROP TABLE plans;` `DROP TABLE usage_counters;`
2. Grep to confirm zero non-test references remain before merging; fix any test
   fixtures that still insert `plan_id`/`usage_counters` (they were only test code).
3. `cargo sqlx prepare` — cache should be clean (PR2 removed all queries against
   these); commit if it changes.

**Tests:** a fresh DB migrates `0001..0034` cleanly end to end; the full backend
suite passes with the tables gone.

**CI/deploy:** must land **after PR2 is live**. **Done when:** deploy C green; the
three objects are gone from prod; full suite green.

---

## Cross-cutting

- **`.sqlx` cache** is load-bearing (required CI check + prod `SQLX_OFFLINE`
  build). Regenerate in PR2 (new queries) and PR3 (dropped tables) against a
  migrated `:5544` DB: `cd backend && cargo sqlx prepare --workspace -- --tests`.
- **Pre-commit hook** needs `DATABASE_URL` (5544) exported for its sqlx check when
  `.rs` files are staged.
- **Local deps** for tests: Postgres `:5544` (`docker start chakra-sqlx-pg`) +
  Redis `:6379` (`docker start chakra-test-redis`) with `REDIS_TEST_URL` set.
- **Rollback:** contract (PR3) is destructive/forward-only → restore from backup.
  PR1/PR2 are reversible by reverting + redeploy (data seeded in PR1 is harmless if
  PR2 is reverted).

## Verification after each deploy
Confirm the per-commit CD run exists and its `Restart relay` + `Probe public
health` steps are green (`gh run view <id>`), then spot-check the DB
(`credit_balance_mc` present PR1; drawing down PR2; tables gone PR3).

## Out of scope (later phases)
Owner visibility (P2), admin management (P3), Dodo purchasing — managed-only (P4),
request-increase + alerts (P5), agent signaling (P6). Phase 1 only stores
`HOSTING_MODE`; it gates nothing yet.
