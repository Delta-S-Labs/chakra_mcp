# Credit ledger foundation — design (Phase 1 of quota productization)

_Drafted 2026-09-23; revised 2026-09-25 for **fully asynchronous credit
accounting**. Status: proposed. Supersedes the quota model from
`2026-07-23-usage-quotas-rate-limiting-design.md` (the "prior quota project"),
which is live and enforcing in prod._

_Phase labels **P1–P6** refer to the phases of quota productization (P1 = this
spec). The superseded project is always the "prior quota project," never "P5"._

## Context

The prior quota project shipped plan-based quotas: named plans
(`free`/`pro`/`enterprise`) carrying `rate_limit_per_min` +
`monthly_invocation_quota`, enforced at the four invocation surfaces (A2A push
via `forwarder`, A2A pull via `inbox_bridge`, legacy `/v1/invoke`, MCP proxy)
through the shared `limits::enforce` primitive. It is live with
`LIMITS_ENFORCE=true`; every account is on `free`.

We replace the **monthly invocation quota** with **credits**. Rate limiting
(velocity, Redis) is orthogonal and stays. This spec is **Phase 1: the credit
foundation** — metering, balances, grants, blocking. Later phases build on it.

### The core constraint: nothing credit-related on the hot path

Credit management must not add latency to invocations. All credit work — charging,
granting, deciding who is blocked — happens in a **background worker**. The
invocation path only consults an **in-memory blocked-accounts set**. Balances may
dip slightly negative between worker sweeps; that is accepted.

### Decisions (locked)

| Decision | Choice |
|----|----|
| Model | Credit-based; replaces the monthly invocation quota. **One substrate for both hosting modes.** |
| Accounting | **Asynchronous.** Zero credit DB work on the invocation path; a background worker charges, grants, and computes blocking. Small negative balances between sweeps are accepted. |
| Unit | Integer **milli-credits (mc)**; 1 credit = 1,000 mc. No floats. |
| Cost | Reads are never metered. Each **accepted invocation** costs a flat `CREDITS_COST_PER_INVOCATION_MC` (default **100** = 0.1 credit), whatever its outcome — identical to the prior quota's "counts every attempt" semantics. Unfair charges (e.g. during an outage) are corrected manually. |
| Free credits | A monthly free grant for every account; **rolls over, uncapped for now.** A cap can be added later with no schema change (skip the grant while balance ≥ cap). |
| Purchased credits | Phase 4 via **Dodo Payments**, managed-only. |
| Refunds | **Out of scope.** Money refunds are issued manually in Dodo; credit corrections are manual `adjustment` ledger rows (P3 tooling). No refund automation → **a single balance per account**, no free/paid buckets. |
| Plans | **Retired.** Rate limit + free grant become per-account overrides on the wallet, falling back to global defaults. |
| Hosting modes | `HOSTING_MODE` = `managed` \| `self_hosted` (default `self_hosted`). Gates purchasing (P4) only. **Nobody self-hosts yet**; self-host hardening is deferred to [a checklist](#first-self-hoster-checklist-deferred). |

## Goals / non-goals

**Goals:** a per-account credit balance; a monthly free grant that rolls over;
per-invocation charging and blocking that replace the monthly-quota check — with
**no added hot-path latency** (in fact less than today); an audit ledger for every
credit-in/adjustment; a per-invocation charge record; retire
`plans`/`usage_counters`/`plan_id` without a downtime window on the live system.

**Non-goals (later phases):** owner visibility (P2), admin credit management
incl. manual corrections (P3), Dodo purchasing (P4), request-increase + alerts
(P5), agent 429 signaling (P6), per-action cost weighting, metering non-invocation
writes, refund automation, free/paid buckets, a free-balance cap.

## Architecture

### Hot path (per invocation)

1. **Rate limit** — unchanged: one Redis `INCR`, fail-open. It must stay inline to
   stop bursts, and it is what bounds the negative drift below. The per-account
   limit comes from an **in-memory override cache** (no DB read).
2. **Credit gate** — `blocked.contains(account_id)` against an **in-memory set**.
   No DB read.
3. **Row write** — the existing `relay_invocations` INSERT, plus one extra bound
   value: `charged_account_id` (the account id each site already holds). No credit
   write, no extra round-trip.

**Latency vs today.** The live `check()` does two DB reads before dispatch
(`resolve_plan`, then the `usage_counters` read), and each metered row write is
wrapped in a transaction solely to upsert `usage_counters`. All of that is
removed. The only remaining I/O is the Redis rate check that already exists.

### Background worker

A tokio task in the relay process (spawned by both the `relay` and `server`
mains), ticking every `CREDITS_SWEEP_INTERVAL_SECS` (default **5**).

Each tick, **one instance** runs the accounting steps, in this order. **Every
batch and step is its own short transaction**, each gated by
`pg_try_advisory_xact_lock` on a fixed key (skip if not acquired) — row locks are
never held across batches, so a hot-path UPDATE on a row being stamped waits at
most one batch statement:

1. **Charge.** Take a bounded batch (e.g. 1,000) of rows
   `WHERE charged_account_id IS NOT NULL AND credits_charged_mc IS NULL`, stamp
   `credits_charged_mc = cost` on each, and apply per-account totals to wallets —
   **one statement**, so charges and balances can never diverge:
   ```sql
   WITH batch AS (
     SELECT id FROM relay_invocations
      WHERE charged_account_id IS NOT NULL AND credits_charged_mc IS NULL
      ORDER BY created_at LIMIT $batch
      FOR UPDATE SKIP LOCKED
   ), stamped AS (
     UPDATE relay_invocations r SET credits_charged_mc = $cost
       FROM batch b WHERE r.id = b.id
     RETURNING r.charged_account_id AS account_id
   ), totals AS (
     SELECT account_id, COUNT(*) * $cost AS total FROM stamped
      WHERE EXISTS (SELECT 1 FROM accounts a WHERE a.id = stamped.account_id)
      GROUP BY account_id
   )
   INSERT INTO credit_wallets (account_id, balance_mc)
   SELECT account_id, -total FROM totals
   ON CONFLICT (account_id)
   DO UPDATE SET balance_mc = credit_wallets.balance_mc + EXCLUDED.balance_mc;
   ```
   The upsert also creates wallets for brand-new accounts (they start negative
   with `free_grant_period` NULL and are granted in step 2 of the same tick).
   Loop while a full batch was processed, within a time budget.
2. **Grant.** For every wallet with `free_grant_period IS NULL OR < current_month`:
   add `months × COALESCE(monthly_free_grant_mc, default)` (months = 1 for NULL,
   else the month difference, capped at **12** — relevant only after a long
   worker outage), advance `free_grant_period`, and write a `free_grant` ledger
   row — one set-based statement (`UPDATE … RETURNING` feeding the ledger
   `INSERT`). Every wallet is granted monthly, active or dormant, so balances are
   always current. Month difference is exact integer arithmetic on first-of-month
   dates.

Then **every instance** refreshes its caches:

3. **Refresh.** `blocked` = wallets
   `WHERE NOT unlimited AND free_grant_period IS NOT NULL AND balance_mc < cost`
   (the NULL-period exclusion means a just-created, not-yet-granted wallet is never
   falsely blocked); `rate_overrides` = wallets with a non-NULL
   `rate_limit_per_min`. Swapped in atomically (e.g. `ArcSwap`/`RwLock`).

`unlimited` affects **only** step 3: unlimited wallets are still charged and
granted (so the ledger invariant holds universally), but never blocked.

### Bounds and failure modes

- **Negative drift.** An exhausted account keeps invoking until the next refresh
  sees it: at most ~one sweep interval of traffic (plus charge lag), bounded by its
  rate limit. On the default 60/min that is ~5–10 invocations ≈ 0.5–1 credit.
- **Worker stalls or crashes.** Invocations keep flowing (fail-open, like the rate
  limiter); unprocessed rows accumulate and are charged on recovery — no lost
  charges, only delayed blocking. Each step is a single atomic statement.
- **Process start.** The blocked set starts empty (nobody blocked) until the first
  refresh, ≤ one tick.
- **Top-ups / admin grants (P3/P4)** take effect at the next refresh (≤ 5 s);
  an in-process refresh can be triggered immediately where convenient.
- **Row contention.** The charge UPDATE takes brief row locks; a pull-mode claim on
  the same row waits at most one short statement. Every metered row is written
  twice (insert + charge stamp) — off the hot path, cleaned by autovacuum. A
  watermark-based design would avoid the second write but risks missing
  out-of-order commits; exactness wins for money.

## Charging rules

- **Who pays** — `charged_account_id` is set at the five metered write sites to the
  same account the prior quota charged:

  | Site | Surface | `charged_account_id` |
  |----|----|----|
  | `forwarder::persist_invocation` | A2A push | `authz.caller_account_id` |
  | `inbox_bridge::park` | A2A pull | `authz.caller_account_id` |
  | `invoke_trusted` | `/v1/invoke` | `row.grantee_account_id` |
  | `invoke_public` | `/v1/invoke` | `invoker.account_id` |
  | `mcp::invoke` | MCP | `row.grantee_account_id` |

- **What's never charged** — any row without `charged_account_id`: all history
  written before this ships, and pre-dispatch rejections recorded outside the five
  sites. So the first sweep cannot bill the past, with no cutover timestamp.
- **When** — within ~one sweep of the row being written. Pull-mode rows (inbox and
  MCP write `pending` rows at queue time) are charged **at admission**, not on
  completion — there is no expiry sweep for stuck `pending` rows, so charging on
  completion would let a caller queue unbounded work against a slow target.
- **Price** — the cost in effect at charge time (≤ one sweep after the invocation)
  is stamped on the row, so later price changes never rewrite history.

## Data model

### `credit_wallets` (new)

```sql
CREATE TABLE credit_wallets (
    account_id            UUID        PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    balance_mc            BIGINT      NOT NULL DEFAULT 0,
    monthly_free_grant_mc BIGINT,                 -- NULL → CREDITS_DEFAULT_MONTHLY_FREE_MC
    rate_limit_per_min    INTEGER,                -- NULL → LIMITS_DEFAULT_RATE_PER_MIN
    free_grant_period     DATE,                   -- first-of-month of the last grant; NULL = never granted
    unlimited             BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()   -- set explicitly by writers (no trigger)
);
CREATE INDEX credit_wallets_grant_period_idx ON credit_wallets (free_grant_period);
```

A separate table rather than columns on `accounts`: billing writes never touch the
`accounts` row, never fire its `accounts_updated_at` trigger, and the billing domain
stays self-contained.

### `credit_ledger` (new, append-only — credits-in and adjustments only)

```sql
CREATE TABLE credit_ledger (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),   -- pgcrypto (0001)
    account_id       UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    delta_mc         BIGINT      NOT NULL,                                -- signed
    reason           TEXT        NOT NULL CHECK (reason IN
                       ('free_grant','purchase','admin_grant','adjustment')),
    external_ref     TEXT,                                                -- external system id, e.g. Dodo payment id
    balance_after_mc BIGINT      NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    metadata         JSONB                                                -- e.g. acting admin, note
);
CREATE INDEX credit_ledger_account_created_idx ON credit_ledger (account_id, created_at DESC);
CREATE UNIQUE INDEX credit_ledger_purchase_ref_uniq ON credit_ledger (external_ref)
    WHERE reason = 'purchase';   -- a retried Dodo webhook can never double-credit (P4)
```

Consumption is not ledgered — it lives per invocation in `relay_invocations`.
Admin identity goes in `metadata`, not `external_ref`. Invariant, for every
account: `balance_mc == Σ credit_ledger.delta_mc − Σ relay_invocations.credits_charged_mc`.

### `relay_invocations` (two new nullable columns + a partial index)

```sql
ALTER TABLE relay_invocations
  ADD COLUMN charged_account_id UUID,      -- set on the hot path; no FK, keeps the INSERT lean
  ADD COLUMN credits_charged_mc BIGINT;    -- stamped by the worker; NULL = not yet charged
CREATE INDEX relay_invocations_uncharged_idx ON relay_invocations (created_at)
  WHERE charged_account_id IS NOT NULL AND credits_charged_mc IS NULL;
```

Adding nullable columns is metadata-only (no table rewrite). The partial index
holds only the unprocessed backlog, so it stays tiny. No FK on
`charged_account_id` (the value comes from a validated auth context; the worker
skips ids whose account no longer exists).

### Dropped (contract migration)

`plans`, `accounts.plan_id`, `usage_counters` — all used only by the quota path
being replaced (`usage::summary` reads `relay_invocations`, not `usage_counters`).

## Enforcement integration

`limits::enforce` keeps its role and stays un-bypassable across the four enforce
call sites (`a2a.rs`, `invoke.rs` ×2, `mcp.rs`), but no longer touches the DB:
rate check (Redis, with the per-account limit from the override cache) → credit
gate (`blocked` set). Shadow mode (`LIMITS_ENFORCE=false`) is unchanged: a
blocked account logs `limit.would_block` and is allowed. Shadow suppresses only
the *deny* — the worker charges and grants regardless.

At the five write sites: delete `quota::increment` and the transaction wrapper
where it existed only for that increment; add `charged_account_id` to the INSERT.

Module shape (relay `src/limits/`): `quota.rs` → removed; new `credits/` with the
worker (charge, grant, refresh) and the cache type; `resolve_plan`/`PlanLimits`
removed; `LimitOutcome::QuotaExceeded` → `InsufficientCredits`. `RelayState`
already carries `Arc<SharedConfig>`; it gains the shared cache handle.

**Error codes — per-surface mapping unchanged, only variant/code/message renamed:**
- **A2A:** `DenyReason::QuotaExceeded` → `InsufficientCredits`, keeping jsonrpc
  `-32008` (so the existing `-32007 | -32008 => TOO_MANY_REQUESTS` arm in the real
  `jsonrpc_to_http` needs no change), data code `chk.limit.credits`, message
  "insufficient credits".
- **`/v1/invoke`:** `ApiError::QuotaExceeded` → `InsufficientCredits`: 429,
  `account_credits_exhausted`.
- **MCP:** JSON-RPC `ERR_INVALID_REQUEST` (a client-side back-off error, **not** a
  429) with the "insufficient credits" message.
- `RateLimited` unchanged.

## Config

Added to `chakramcp_shared::config::SharedConfig` (every construction site —
`from_env` and the server's `load_config` — must be updated to compile):

| Env | Default | Meaning |
|----|----|----|
| `HOSTING_MODE` | `self_hosted` | `managed` \| `self_hosted`. Stored only; gates purchasing in P4. |
| `CREDITS_DEFAULT_MONTHLY_FREE_MC` | `100000` (100 credits) | Monthly grant when `monthly_free_grant_mc` is NULL. At 0.1/invocation = **1,000 invocations/mo = today's free tier.** |
| `CREDITS_COST_PER_INVOCATION_MC` | `100` (0.1 credit) | Flat cost per accepted invocation. |
| `CREDITS_SWEEP_INTERVAL_SECS` | `5` | Worker tick. |
| `LIMITS_DEFAULT_RATE_PER_MIN` | `60` | Rate limit when `rate_limit_per_min` is NULL. |

`LIMITS_ENFORCE` stays as today (read in main via `enforce_flag`). The migration
seeds no balances, so there is no migration/env coupling.

## Migration & cutover — expand / swap / contract

CD runs migrations **before** restarting the relay, so the old relay keeps
serving during the migrate step. Three PRs, three deploys, merged one at a time
(CD's concurrency group drops a middle deploy if merges land in a burst).

**`0033_credits_expand.sql` (PR1 — additive):**
0. **Guard:** `RAISE EXCEPTION` if any account is on a non-`free` plan — the credit
   system won't honor tiers, so refuse to run rather than silently downgrade
   (a failed migrate step leaves the old relay running — a safe failure).
1. `CREATE TABLE credit_wallets` + index.
2. `CREATE TABLE credit_ledger` + indexes.
3. `ALTER TABLE relay_invocations ADD` the two columns; create the partial index
   (existing rows don't match it; a plain build is fine at current table size —
   use `CONCURRENTLY` in a no-transaction migration for large deployments).
4. `INSERT INTO credit_wallets (account_id) SELECT id FROM accounts;` — balance 0,
   period NULL; the worker's first tick (after PR2) grants every account.
- Nothing existing is altered or dropped; the old relay is unaffected.

**PR2 (swap — no migration):** ship the worker, caches, hot-path changes, error
renames. First tick after deploy: grants every wallet one month (everyone starts
with a fresh 100 credits — current-month usage under the old quota is not carried
over); charging begins with the first rows carrying `charged_account_id`.

**`0034_credits_contract.sql` (PR3, after PR2 is live):** drop `accounts.plan_id`,
`plans`, `usage_counters`. Safe: no live code references them.

Forward-only for the contract step (rollback = restore from backup). PR1/PR2 revert
cleanly.

**Rollout.** Optional safety bake: set `LIMITS_ENFORCE=false` before PR2, watch
the worker's per-tick logs (charged rows, grants, blocked count, backlog) against
the DB, then re-enable.

**Observability.** Each tick logs rows charged, accounts touched, grants applied,
blocked-set size, remaining backlog, and duration; warn when the backlog grows
across consecutive ticks (the worker is falling behind).

## Testing

- **Charge:** rows with `charged_account_id` get stamped and wallets decremented by
  the per-account total; rows without it (history, pre-dispatch rejections) are
  never touched; a second sweep charges nothing new; two concurrent sweeps never
  double-charge (advisory lock + `SKIP LOCKED`); a wallet is created for a
  brand-new account; ids whose account is gone are skipped.
- **Grant:** NULL period → one month + a `free_grant` ledger row; month rollover →
  one grant; worker-outage catch-up capped at 12; unlimited wallets still granted;
  re-running within a month grants nothing.
- **Refresh:** below cost + granted + not unlimited → blocked; never-granted wallet
  not blocked; unlimited never blocked; rate overrides loaded.
- **Enforce:** blocked → deny (enforce) / `limit.would_block` + allow (shadow);
  unblocked → allow; per-account rate override honored.
- **Five write sites:** each writes the expected `charged_account_id` and does no
  synchronous credit write; exhausted account denied per surface — A2A and
  `/v1/invoke` → **429** (`account_credits_exhausted`), MCP → **JSON-RPC
  `ERR_INVALID_REQUEST`** with "insufficient credits".
- **Invariant:** after a sequence of grants and charges,
  `balance == Σ ledger − Σ charges` for every account.
- **Migration:** the 0033 guard fires on a non-free account; wallets exist for every
  account; a fresh DB migrates 0001→0034 cleanly.
- **Live Redis** rate path stays green (existing `backend-ci` Redis service).

## Later phases

- **P2 — Owner visibility:** balance (`credit_wallets`), spend (`relay_invocations.
  credits_charged_mc`), grants and adjustments (`credit_ledger`), effective limits.
  Accounts created after PR1 that haven't invoked yet have no wallet (one is created
  at their first charge and granted that tick) — P2 shows the default grant for
  them, or creates wallets eagerly if that proves simpler.
- **P3 — Admin credit management:** grant, adjust (incl. manual refund
  corrections), set per-account grant/rate/unlimited; admin API + UI; every change a
  ledger row with the acting admin in `metadata`.
- **P4 — Purchasing (managed-only, Dodo):** gated by `HOSTING_MODE=managed`;
  checkout + signed webhooks → `purchase` ledger rows (deduped by the unique index)
  + balance top-up. Refunds stay manual.
- **P5 — Request-increase + alerts:** low-balance alerts computed by the worker
  tick; users request more allowance from their admin.
- **P6 — Agent signaling:** 429 `Retry-After` + balance/limit headers.
- **Cost weighting:** the worker stamps the price per row, so per-class pricing is a
  change to the stamp computation only — the hot path is unaffected.
- **Free-balance cap:** skip the monthly grant while `balance_mc ≥ cap` — one
  config knob, no schema change.

## First-self-hoster checklist (deferred)

Nobody self-hosts today. Before the first one does:
- Make `infra/docker-compose.prod.yml` defaults self-host-safe (observe-only), moving
  managed values into the operator's host `.env` — ⚠️ set `HOSTING_MODE=managed` (or
  `LIMITS_ENFORCE=true`) in `/opt/chakramcp/.env` **before** that compose change
  deploys, or the managed service silently drops to shadow.
- Expose the new knobs in `server.toml` (`ServerFile` + `load_config`) and the
  INSTALL.md config table.
- `chakramcp-server credits {show,grant,set-limit,set-unlimited}` escape hatch.
- Upgrade note: stop the relay before migrating across the credits cutover.
- INSTALL.md + `/docs/self-host`: hosting modes and credit settings.

## Resolved defaults

100 credits/month free grant (rolls over, uncapped) · 0.1 credit per accepted
invocation · 60 req/min · 5 s sweep · 12-month outage catch-up cap · every accepted
attempt is charged · single balance · `usage_counters` dropped outright.
