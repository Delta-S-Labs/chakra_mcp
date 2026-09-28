# Credit ledger foundation — design (Phase 1 of quota productization)

_Drafted 2026-09-23. Revised 2026-09-25: fully asynchronous accounting, charge
queue, review fixes. Status: proposed. Supersedes the quota model from
`2026-07-23-usage-quotas-rate-limiting-design.md` (the "prior quota project"),
which is live and enforcing in prod._

_Phase labels **P1–P6** refer to the phases of quota productization (P1 = this
spec). The superseded project is always the "prior quota project," never "P5"._

## Context

The prior quota project shipped plan-based quotas: named plans
(`free`/`pro`/`enterprise`) carrying `rate_limit_per_min` +
`monthly_invocation_quota`, enforced at the four invocation surfaces (A2A push via
`forwarder`, A2A pull via `inbox_bridge`, legacy `/v1/invoke`, MCP proxy) through
the shared `limits::enforce` primitive. It is live with `LIMITS_ENFORCE=true`;
every account is on `free`.

We replace the **monthly invocation quota** with **credits**. This spec is
**Phase 1: the credit foundation** — metering, balances, grants, blocking.

### The governing rule: never hamper invocation performance

The invocation path only **reads switches** held in memory. The credit system
**runs in parallel** — a background worker charges, grants, and flips the
switches. Balances may overflow slightly negative between worker ticks; that is
the accepted price of zero added latency.

### Decisions (locked)

| Decision | Choice |
|----|----|
| Model | Credit-based; replaces the monthly invocation quota. One substrate for both hosting modes. |
| Accounting | **Asynchronous.** The hot path does in-memory switch reads plus one small queue row inside the INSERT it already runs. Everything else is the worker's job. |
| Unit | Integer **milli-credits (mc)**; 1 credit = 1,000 mc. No floats. |
| Cost | Reads never metered. Each **accepted invocation** costs a flat `CREDITS_COST_PER_INVOCATION_MC` (default **100** = 0.1 credit) at admission, whatever its outcome — the prior quota's "counts every attempt" semantics. Unfair charges are corrected manually. |
| Free credits | Monthly free grant; **rolls over, uncapped for now** (a cap later = skip the grant while balance ≥ cap; no schema change). |
| Purchased credits | P4 via **Dodo Payments**, managed-only. |
| Refunds | **Out of scope.** Money refunds are manual in Dodo; credit corrections are manual `adjustment` ledger rows (P3). Single balance — no free/paid buckets. |
| Plans | **Retired.** Rate limit + free grant become per-account overrides on the wallet, with global defaults. |
| Rate limiting | **Stays on Redis** (inline `INCR`, fail-open) — decided 2026-09-25. |
| Hosting modes | `HOSTING_MODE` is introduced in **P4**, not here (the prod compose *is* the managed deployment; defaulting it to `self_hosted` there would be a trap). Nobody self-hosts yet — see the [deferred checklist](#first-self-hoster-checklist-deferred). |

## Goals / non-goals

**Goals:** per-account credit balances; a monthly free grant that rolls over;
charging and blocking that replace the monthly-quota check with **less** hot-path
work than today; an audit ledger for credits-in/adjustments; a per-invocation charge
record; retire `plans`/`usage_counters`/`plan_id` without a downtime window.

**Non-goals (later phases):** owner visibility (P2), admin credit management incl.
manual corrections (P3), Dodo purchasing (P4), request-increase + alerts (P5),
agent 429 signaling (P6), per-action cost weighting, metering non-invocation
writes, refund automation, free/paid buckets, a free-balance cap.

## Architecture

### Hot path (per invocation)

1. **Credit switch** — `cache.is_blocked(account_id)`, in memory. A stale cache
   (no successful refresh for > 3 ticks) reports *nobody blocked* — fail open.
   Checked first, so a blocked account costs no Redis call and uses no rate
   budget.
2. **Rate limit** — unchanged: one Redis `INCR`, fail-open. The per-account limit
   comes from an in-memory override cache (no DB read).
3. **Row write** — each metered site's existing `relay_invocations` INSERT becomes
   **one statement** that also enqueues the charge. The id is the app-minted
   UUIDv7 the site already has:
   ```sql
   WITH q AS (
     INSERT INTO credit_charge_queue (invocation_id, account_id) VALUES ($1, $2)
   )
   INSERT INTO relay_invocations (id, …) VALUES ($1, …);
   ```
   A data-modifying CTE always executes, and the whole statement is atomic: the
   queue row exists iff the invocation row does. One round trip.

**Latency vs today.** Today each metered invocation does two DB reads before
dispatch (`resolve_plan`, the `usage_counters` read), then `BEGIN` → INSERT →
`usage_counters` upsert → `COMMIT` (four round trips). After: zero reads, one
statement.

### Background worker

Two **supervised** tokio loops in the relay process (spawned by both the `relay`
and `server` mains), each ticking every `CREDITS_SWEEP_INTERVAL_SECS` (default
**5**):

- **accounting** — steps 1–2 below;
- **switches** — step 3.

They run **independently**, so a stalled charge or grant can never stop the
switches refreshing (which would make them fail open). They share a **pool of
their own** (two connections, the app pool's settings), so the worker never takes
a connection from the invocation path. **Every transaction bounds its lock waits**
with `SET LOCAL lock_timeout = '1s'` (transaction-scoped, so safe behind a
transaction-pooling proxy): a wallet row held elsewhere — an admin mid-edit —
costs that pass, and the work is retried next tick instead of hanging. The worker
never touches `relay_invocations`.

Per accounting pass, **one instance** runs the steps; **each step is its own short
transaction** that first takes `SELECT pg_try_advisory_xact_lock(<key>)` as a
separate statement (skip if not acquired). A failed step's transaction is rolled
back immediately, releasing the lock for the pass's next step.

1. **Charge** — drain up to 1,000 queue rows, record each charge, debit wallets:
   ```sql
   WITH drained AS (
     DELETE FROM credit_charge_queue
      WHERE invocation_id = ANY(ARRAY(SELECT invocation_id FROM credit_charge_queue
                                       ORDER BY invocation_id LIMIT $1
                                       FOR UPDATE SKIP LOCKED))
     RETURNING invocation_id, account_id
   ), charged AS (
     INSERT INTO invocation_charges (invocation_id, account_id, cost_mc)
     SELECT d.invocation_id, d.account_id,
            CASE WHEN EXISTS (SELECT 1 FROM accounts a WHERE a.id = d.account_id)
                 THEN $2::bigint ELSE 0 END           -- deleted account: record 0, debit nothing
       FROM drained d
     ON CONFLICT (invocation_id) DO NOTHING
     RETURNING account_id, cost_mc
   ), totals AS (
     SELECT account_id, SUM(cost_mc)::bigint AS total   -- SUM(bigint) is numeric: cast
       FROM charged WHERE cost_mc > 0 GROUP BY account_id
   )
   INSERT INTO credit_wallets (account_id, balance_mc, updated_at)
   SELECT account_id, -total, now() FROM totals
   ON CONFLICT (account_id) DO UPDATE
     SET balance_mc = credit_wallets.balance_mc + EXCLUDED.balance_mc, updated_at = now();
   ```
   The upsert creates a wallet at an account's first charge (period NULL → granted
   in step 2 of the same pass). Loop while a full batch was drained, within a time
   budget (2 s, never more than one tick). If an account is deleted mid-statement
   the FK fails, the transaction rolls back, and the next pass records that charge
   as 0 — self-healing. `= ANY(ARRAY(…))` runs the row pick once and deletes by
   primary key; with `IN (…)`, PG 16 plans a hash semi-join over a full scan of the
   queue for every batch (checked at a 300k backlog: 22 ms vs 3 ms per batch), which
   makes draining a backlog quadratic.

2. **Grant** — `$1` = the current UTC first-of-month (computed once per tick),
   `$2` = the default grant:
   ```sql
   WITH due AS (
     SELECT account_id,
            (CASE WHEN free_grant_period IS NULL THEN 1
                  ELSE LEAST(12, ((EXTRACT(YEAR FROM $1::date) - EXTRACT(YEAR FROM free_grant_period)) * 12
                                + (EXTRACT(MONTH FROM $1::date) - EXTRACT(MONTH FROM free_grant_period)))::int)
             END)::bigint * COALESCE(monthly_free_grant_mc, $2::bigint) AS delta
       FROM credit_wallets
      WHERE free_grant_period IS NULL OR free_grant_period < $1::date
      FOR UPDATE
   ), upd AS (
     UPDATE credit_wallets w
        SET balance_mc = w.balance_mc + due.delta, free_grant_period = $1::date, updated_at = now()
       FROM due
      WHERE w.account_id = due.account_id
        AND (w.free_grant_period IS NULL OR w.free_grant_period < $1::date)   -- re-checked under concurrency
     RETURNING w.account_id, due.delta, w.balance_mc
   )
   INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc, metadata)
   SELECT account_id, delta, 'free_grant', balance_mc, jsonb_build_object('period', $1::date) FROM upd;
   ```
   Every wallet is granted once per month, active or dormant; the 12-month cap only
   matters after a long worker outage. (PG 16 has no `RETURNING OLD`, and a naive
   subquery rewrite double-grants under overlapping runs — this shape was tested
   against both.)

On **every instance**, the switches loop:

3. **Refresh** — `blocked` = wallets
   `WHERE NOT unlimited AND free_grant_period IS NOT NULL AND balance_mc < cost`
   (never-granted wallets are never falsely blocked); `rate_overrides` = wallets
   with non-NULL `rate_limit_per_min`. Swapped in atomically with a `refreshed_at`
   stamp. Plain reads never wait on row locks; the lock timeout covers table locks
   (DDL). A failed refresh keeps the last snapshot until it goes stale; the moment
   it does, the loop logs an **error** (enforcement is off until a refresh
   succeeds), and logs recovery.

**Supervision.** Each step's errors are logged and the loop continues; a loop that
panics — or, somehow, returns — is restarted after 1 s (never a tight respawn).
Config is validated at startup: every value parses and is > 0 (a zero interval
would panic `tokio::time::interval`), and the tick is at most 300 s (the fail-open
window is 3× the tick). One refresh runs **synchronously before the server accepts
traffic**, so a deploy never opens a window where blocked accounts are let through.

`unlimited` affects only the switch: unlimited wallets are still charged and
granted (the ledger invariant holds universally), but never blocked.

### Bounds and failure modes

- **Overflow.** An exhausted account keeps invoking until the next refresh — at
  most ~one tick of traffic, bounded by its rate limit (~0.5–1 credit on the
  default 60/min).
- **Worker down.** Invocations are unaffected. The queue grows and is charged on
  recovery (no lost charges). After 3 missed refreshes the switch fails **open**
  (nobody blocked) rather than freezing whoever was blocked.
- **Accounting stalled** (a held wallet lock, a slow statement). Balances stop
  moving, so existing blocks hold and new ones wait — a bounded revenue leak, never
  a fail-open: the switches loop keeps refreshing on its own.
- **No contention with invocations.** The worker never reads or writes
  `relay_invocations`; a queue row is touched only by the worker, after commit; and
  the worker has its own connections.
- **Queue churn.** Insert-then-delete creates dead tuples; the queue table gets
  aggressive autovacuum reloptions (a fixed dead-tuple threshold, not a fraction of
  table size) and `vacuum_truncate = false` (0035): the drain empties the table's
  tail every tick, and truncating it would take an ACCESS EXCLUSIVE lock that the
  hot-path inserts queue behind.
- **Uncharged edges.** A push call whose upstream answers but whose body can't be
  read is still recorded (`failed`) and charged. A call cut off mid-flight —
  handler cancelled, process restarted during the upstream request — writes no row
  and is not charged; the row is written after the upstream returns so the hot path
  stays one statement.
- **Top-ups / admin grants (P3/P4)** take effect at the next refresh (≤ 5 s), or
  immediately via an in-process refresh.

## Charging rules

- **Who pays** — the account each metered site already charges today:

  | Site | Surface | Queue `account_id` |
  |----|----|----|
  | `forwarder::persist_invocation` | A2A push | `authz.caller_account_id` |
  | `inbox_bridge::park` | A2A pull | `authz.caller_account_id` |
  | `authorize_and_enqueue_trusted` | `/v1/invoke` (grant) **and** the MCP `invoke` tool | `row.grantee_account_id` |
  | `invoke_public` | `/v1/invoke` (public capability) | `invoker.account_id` |

  (Since #312 the MCP `invoke` tool shares the trusted path, so there are four
  write sites, not five.)

- **Never charged** — anything that never enqueues: all history before this ships,
  pre-dispatch rejections (`record_terminal`), and legacy `/v1/invoke` calls to
  push-mode agents (rejected before dispatch — a separate prerequisite PR; today
  they park a row no one ever delivers).
- **When** — at admission. Pull-mode rows (`pending` at park) are charged then, not
  on completion — there is no expiry for stuck `pending` rows, so charging on
  completion would let a caller queue unbounded work against a slow target.
- **Price** — the cost in effect when the worker drains the row (≤ one tick after
  the invocation), recorded in `invocation_charges`, so later price changes never
  rewrite history.

## Data model

```sql
-- Created at an account's first charge; no backfill (never-used accounts don't accrue).
CREATE TABLE credit_wallets (
    account_id            UUID        PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    balance_mc            BIGINT      NOT NULL DEFAULT 0,
    monthly_free_grant_mc BIGINT,                 -- NULL → CREDITS_DEFAULT_MONTHLY_FREE_MC
    rate_limit_per_min    INTEGER,                -- NULL → LIMITS_DEFAULT_RATE_PER_MIN
    free_grant_period     DATE,                   -- first-of-month of the last grant; NULL = never granted
    unlimited             BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()   -- set explicitly by writers
);
CREATE INDEX credit_wallets_grant_period_idx ON credit_wallets (free_grant_period);

-- Append-only: credits-in and adjustments. No FK to accounts: an org hard-delete
-- must not erase purchase history or its dedupe keys.
CREATE TABLE credit_ledger (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),   -- pgcrypto (0001)
    account_id       UUID        NOT NULL,
    delta_mc         BIGINT      NOT NULL,
    reason           TEXT        NOT NULL CHECK (reason IN ('free_grant','purchase','admin_grant','adjustment')),
    external_ref     TEXT,                                                -- e.g. Dodo payment id
    balance_after_mc BIGINT      NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    metadata         JSONB,                                               -- acting admin, note, grant period
    CHECK (reason <> 'purchase' OR external_ref IS NOT NULL)              -- NULLs would bypass the unique index
);
CREATE INDEX credit_ledger_account_created_idx ON credit_ledger (account_id, created_at DESC);
CREATE UNIQUE INDEX credit_ledger_purchase_ref_uniq ON credit_ledger (external_ref) WHERE reason = 'purchase';

-- Hot-path insert target: tiny, no FKs, drained every tick.
CREATE TABLE credit_charge_queue (
    invocation_id UUID PRIMARY KEY,
    account_id    UUID NOT NULL
) WITH (autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);

-- One row per charged invocation. No FKs: billing history outlives the audit table
-- and deleted accounts.
CREATE TABLE invocation_charges (
    invocation_id UUID        PRIMARY KEY,        -- double-charging is structurally impossible
    account_id    UUID        NOT NULL,
    cost_mc       BIGINT      NOT NULL,
    charged_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invocation_charges_account_idx ON invocation_charges (account_id, charged_at DESC);
```

`relay_invocations` is **not changed**. Invariant, for every existing account:
`balance_mc == Σ credit_ledger.delta_mc − Σ invocation_charges.cost_mc`.

Credits do **not** meter from `usage_events` (migration 0025 calls it "the billing
substrate", but it records REST-request analytics; billing is `invocation_charges`).

**Dropped (contract migration):** `plans`, `accounts.plan_id`, `usage_counters` —
used only by the quota path being replaced.

## Enforcement integration

`limits::enforce` stays un-bypassable at the three enforce call sites (`a2a.rs`,
and `invoke.rs`'s trusted and public paths — the MCP `invoke` tool goes through
the trusted one) but no longer touches the DB: credit switch → rate check (Redis,
limit from the override cache). Shadow mode (`LIMITS_ENFORCE=false`) is
unchanged: a blocked account logs `limit.would_block` and is allowed; the worker
charges and grants regardless.

At the four write sites, the `BEGIN` → INSERT → `quota::increment` → `COMMIT`
transaction (it wraps exactly those two statements at every site) becomes the single
enqueueing statement above.

Module shape (relay `src/limits/`): `quota.rs` and `resolve_plan`/`PlanLimits` are
removed; a new `credits/` module holds `CreditsConfig`, the cache, and the worker;
`LimitOutcome::QuotaExceeded` → `InsufficientCredits`. `RelayState` gains the cache
and `CreditsConfig` via builders (`RelayState::new` defaults to an empty cache and
default config, so its many test callers stay untouched).

**Error codes — per-surface mapping unchanged, only variant/code/message renamed:**
- **A2A:** `DenyReason::InsufficientCredits`, jsonrpc `-32008` (the existing
  `-32007 | -32008 => TOO_MANY_REQUESTS` arm in `jsonrpc_to_http` needs no change),
  data code `chk.limit.credits`, message "insufficient credits".
- **`/v1/invoke`:** `ApiError::InsufficientCredits` → 429, `account_credits_exhausted`.
- **MCP:** JSON-RPC `ERR_INVALID_REQUEST` (a client-side back-off error, not a 429)
  with "insufficient credits".
- `RateLimited` unchanged. The renamed codes are an external contract change — no
  SDK matches on them, so a release note suffices.

## Config — `CreditsConfig`

A standalone struct with `Default` and `validate()`, parsed from env in both mains
and attached to `RelayState` via a builder. `SharedConfig` is untouched (it has no
`Default` and ~13 test literals).

| Env | Default | Meaning |
|----|----|----|
| `CREDITS_DEFAULT_MONTHLY_FREE_MC` | `100000` (100 credits) | Monthly grant when `monthly_free_grant_mc` is NULL. At 0.1/invocation = **1,000 invocations/mo = today's free tier.** |
| `CREDITS_COST_PER_INVOCATION_MC` | `100` (0.1 credit) | Flat cost per accepted invocation. |
| `CREDITS_SWEEP_INTERVAL_SECS` | `5` | Worker tick. |
| `LIMITS_DEFAULT_RATE_PER_MIN` | `60` | Rate limit when `rate_limit_per_min` is NULL. |

Unset → default; set but unparseable or ≤ 0 → **fail fast at startup** (money
settings must never silently fall back). `LIMITS_ENFORCE` is unchanged.

## Migration & cutover — expand / swap / contract

Both binaries run `sqlx::migrate!` **at boot**, and sqlx refuses to start when the
DB has a migration the binary doesn't know. CD also migrates before restarting.
Four PRs, merged one at a time (CD's concurrency group drops a middle deploy).

**Before merging PR1:** run the guard's query against prod and confirm 0:
`SELECT count(*) FROM accounts a JOIN plans p ON p.id = a.plan_id WHERE p.name <> 'free';`
— because a guard failure at *boot* (outside the CD migrate step) would crash-loop.

**`0034_credits_expand.sql` (PR1 — additive; `0033` is #312's grant purpose):** `SET LOCAL lock_timeout`; guard
(`RAISE EXCEPTION` if any account is on a non-free plan); create `credit_wallets`,
`credit_ledger`, `credit_charge_queue`, `invocation_charges` + indexes. Touches
neither `relay_invocations` nor `accounts`; no backfill.

**`0035_credits_hardening.sql` (PR1b — additive, live before PR2):**
`credit_charge_queue SET (vacuum_truncate = false)` — the worker drains the queue
to empty every tick, so VACUUM would otherwise truncate its tail pages each run
under an ACCESS EXCLUSIVE lock that the hot-path inserts queue behind; plus
`CHECK (rate_limit_per_min > 0)` and `CHECK (monthly_free_grant_mc >= 0)`.

**PR2 (swap — no migration):** worker, cache, config, the four enqueueing
statements, error renames. First tick after deploy: wallets appear as accounts
invoke; each is granted one month (everyone starts fresh — current-month usage
under the old quota is not carried over).

**`0036_credits_contract.sql` (PR3, after PR2 is live):** drop `accounts.plan_id`,
`plans`, `usage_counters`. **Must-succeed deploy:** once 0036 is applied, the PR2
binary can no longer boot.

**Rollback.** Revert *code*, never migration files — a binary missing an applied
migration refuses to boot (or ship the revert with `set_ignore_missing(true)`).
PR1's tables are inert without PR2. PR3 is forward-only → restore from backup.

**Rollout.** Optional safety bake: set `LIMITS_ENFORCE=false` before PR2, watch the
worker's tick logs, re-enable.

**Observability.** Each accounting pass logs rows drained, wallet debits, grants,
queue depth, and duration; warn when the queue grows across consecutive passes.
The switches loop logs when the blocked-set size changes, an error when the
switches go stale, and their recovery.

## Testing

- **Charge:** each queued row drained exactly once; charges recorded at `cost`;
  wallets debited by per-account totals; a wallet created at an account's first
  charge; a deleted account's charge recorded as 0 with no debit; re-running is a
  no-op; a backlog drains across batches; while one drain holds rows, a second
  (without the advisory lock, under a short `lock_timeout`) takes the rest instead
  of waiting (`SKIP LOCKED`); a second instance skips a pass while the advisory
  lock is held.
- **Grant:** NULL → 1 month; same month → no-op; rollover; a 34-month gap → 12;
  exact `balance_after_mc`; two overlapping runs don't double-grant.
- **Switch/refresh:** blocked cases (incl. never-granted and unlimited); rate
  overrides load; a stale cache reports nobody blocked; failed refreshes keep the
  snapshot until stale, then fail open, then recover; the startup refresh runs
  before serving.
- **Lock bounds:** a wallet row held by another session costs the charge one pass
  (lock timeout), grants still land, the switches still refresh, and the next pass
  charges it.
- **Supervision/config:** a loop that panics or returns is restarted after a
  pause; 0, unparseable, or over-300 s values fail startup.
- **Enforce:** deny / shadow-allow / per-account rate override.
- **Four sites:** each writes exactly one queue row with the expected account in the
  same statement; exhausted denial per surface (A2A + `/v1/invoke` → 429, MCP →
  JSON-RPC error).
- **Invariant:** `balance == Σ ledger − Σ charges` after mixed grants and charges.
- **Migration:** the guard test uses `#[sqlx::test(migrations = false)]` with
  `Migrator::run_to(33)` then `run_to(34)` and runtime `sqlx::query` (so `.sqlx`
  stays unchanged; it's the one allowed `plans` reference after PR2); a fresh DB
  migrates `0001→0036` cleanly.
- **Live Redis** rate path stays green.

## Prerequisites (decided 2026-09-25, outside credits) — shipped

- **PR-A — usage middleware off the hot path** ([#323](https://github.com/Delta-S-Labs/chakra_mcp/pull/323)).
  The relay's middleware no longer re-authenticates each request before the
  handler or awaits a `usage_events` INSERT before returning; a background writer
  fed by a bounded, never-blocking channel does both.
- **PR-B — refuse queued invocations to push-mode agents** ([#324](https://github.com/Delta-S-Labs/chakra_mcp/pull/324)).
  `/v1/invoke` and the MCP `invoke` tool return 409 before dispatch instead of
  parking a row no one delivers, so they're never charged.

## Later phases

- **P2 — Owner visibility:** balance (`credit_wallets`; accounts without one show
  the default grant), spend (`invocation_charges`), grants/adjustments
  (`credit_ledger`), effective limits.
- **P3 — Admin credit management:** grant, adjust (incl. manual refund corrections),
  per-account overrides, the `unlimited` switch; admin API + UI; each change a
  ledger row with the acting admin in `metadata`.
- **P4 — Purchasing (managed-only, Dodo):** introduces `HOSTING_MODE` (default
  `managed` in the prod compose); checkout + signed webhooks → `purchase` rows
  (deduped by the unique index) + top-up. Refunds stay manual.
- **P5 — Request-increase + alerts:** low-balance alerts computed on the worker tick.
- **P6 — Agent signaling:** 429 `Retry-After` + balance/limit headers.
- **Cost weighting:** the worker prices each drained row, so per-class pricing
  changes only the worker — never the hot path.
- **Free-balance cap:** skip the monthly grant while `balance_mc ≥ cap`.

## First-self-hoster checklist (deferred)

Nobody self-hosts today. Before the first one does: self-host-safe compose
defaults (managed values moved to the operator's host `.env` — set them there
*before* the compose change deploys); the credit knobs in `server.toml`; a
`chakramcp-server credits {show,grant,set-limit,set-unlimited}` escape hatch; an
upgrade note (stop the relay before migrating across the cutover); INSTALL.md and
`/docs/self-host`.

## Resolved defaults

100 credits/month free grant (rolls over, uncapped) · 0.1 credit per accepted
invocation · 60 req/min · 5 s tick · 12-month outage catch-up cap · every accepted
attempt charged · single balance · Redis rate limiting · `usage_counters` dropped.
