# Credit ledger foundation — design (Phase 1 of quota productization)

_Drafted 2026-09-23. Status: proposed. Supersedes the quota model from
`2026-07-23-usage-quotas-rate-limiting-design.md` (the "prior quota project"),
which is live and enforcing in prod._

_Phase labels **P1–P6 in this document refer to the phases of quota
productization** (P1 = this spec). The superseded project is always called the
"prior quota project," never "P5", to avoid a collision._

## Context

The prior quota project shipped a plan-based quota system: named plans (`free`/`pro`/`enterprise`)
carrying `rate_limit_per_min` + `monthly_invocation_quota`, enforced across the
four invocation surfaces (A2A push via `forwarder`, A2A pull via `inbox_bridge`,
legacy `/v1/invoke`, MCP proxy) through the shared `limits::enforce` primitive.
It is live and enforcing (`LIMITS_ENFORCE=true`) with placeholder limits;
everyone is on `free`.

We are replacing the **monthly invocation quota** with a **credit** model.
Rate limiting (velocity, req/min, Redis) is orthogonal and stays. This document
specs **Phase 1: the credit-ledger foundation** — the metering substrate. Later
phases build on it (see [Later phases](#later-phases)).

### Deployment modes (managed vs self-hosted)

ChakraMCP is self-hostable, so the credit substrate serves two modes, gated by a
`HOSTING_MODE` config (`managed` | `self_hosted`, default **`self_hosted`** so a
fresh self-host never exposes purchasing):

- **Managed** (the operator's own hosted instance): credits are mostly
  **purchased** (Dodo, Phase 4); the superadmin can also grant.
- **Self-hosted** (anyone running their own instance): **no payment.** The
  instance's own admin grants allowance, relies on the monthly auto-renewal (the
  free grant), fields user "increase my quota" requests, and observes usage /
  receives alerts.

**The enforcement substrate is identical in both modes** (per-account balance,
ledger, consume, enforce). Only what tops the balance up and the surrounding UX
differ:

| Capability | Managed | Self-hosted |
|----|----|----|
| Enforcement + balance (**this phase**) | ✓ | ✓ |
| Monthly free grant / auto-renewal (**this phase**) | ✓ | ✓ |
| Admin grants (P3) | ✓ (operator/superadmin) | ✓ (their admin) |
| Purchasing (P4, Dodo) | ✓ | ✗ (gated off by `HOSTING_MODE`) |
| Request-increase + usage alerts (P5) | ✓ | ✓ (primary top-up path) |

Phase 1 is the mode-agnostic substrate; it introduces the `HOSTING_MODE` config
but gates nothing yet (purchasing arrives in P4). UI terminology ("credits / buy"
vs "allowance / request increase") is a per-mode presentation detail for later
phases.

### Decisions locked in brainstorming

| Decision | Choice |
|----|----|
| Model | Credit-based, replaces the monthly invocation quota; **one universal substrate for both hosting modes.** |
| Hosting modes | `HOSTING_MODE` = `managed` \| `self_hosted` (default `self_hosted`); gates purchasing (P4) only. |
| Free credits | Every account gets a monthly free grant that **rolls over** (accumulates; no expiry). |
| Purchased credits | Persist; added later (Phase 4, via **Dodo Payments**). |
| Plans | **Retired.** Rate limit + free grant move to per-account columns with a global default. |
| Credit cost | **Reads free; invocations cost a flat fraction.** Weighting-ready but a single flat cost for now. |
| Unit | Integer **milli-credits (mc)**; 1 credit = 1,000 mc. No floats (credits become money). |
| Storage | Materialized `accounts.credit_balance_mc` + append-only `credit_ledger` for credits-in/adjustments; consumption decrements the balance (detail already in `relay_invocations`). |

## Goals / non-goals

**Goals:** a per-account credit balance in mc; a monthly free grant that rolls
over; per-invocation draw-down replacing the monthly-quota check in
`limits::enforce`; an audit ledger for every credit-in/adjustment; a
mode-agnostic substrate that serves both hosting modes (introduce `HOSTING_MODE`
config); retire the `plans`/`usage_counters`/`plan_id` machinery; do it without a
downtime window on the live enforcing system.

**Non-goals (later phases):** owner-facing visibility (P2), admin management
(P3), Dodo purchasing — managed-only (P4), request-increase + alerts (P5), agent
429 signaling (P6), per-action-class cost weighting, metering non-invocation
platform writes, and any purchasing/request/alert UX (Phase 1 only stores the
`HOSTING_MODE` flag; it gates nothing yet).

## The credit model

- **Unit.** All quantities (balance, grant, cost) are integer **milli-credits
  (mc)**; `1 credit = 1_000 mc`. Fractional costs are exact.
- **Cost.** Reads are never metered. Each **invocation** costs a flat
  `CREDITS_COST_PER_INVOCATION_MC` (default **100** = 0.1 credit). A single
  `cost_mc()` lookup returns this today; it can become a per-action-class table
  later without a schema change. The `consume(cost_mc)` primitive already takes
  a cost argument.
- **Balance.** One persistent per-account balance (mc). Free grant rolls over,
  purchases/admin-grants persist — all additive; only invocations draw down.
  Balance is never auto-reset.

## Data model

### `accounts` — new columns

```sql
ALTER TABLE accounts
  ADD COLUMN credit_balance_mc      BIGINT  NOT NULL DEFAULT 0,
  ADD COLUMN monthly_free_grant_mc  BIGINT,            -- NULL → global default
  ADD COLUMN rate_limit_per_min     INTEGER,           -- NULL → global default (moved off plans)
  ADD COLUMN free_grant_period      DATE;              -- last month the free grant was applied
```

- `credit_balance_mc` — materialized balance; O(1) check in `enforce`,
  decremented per invocation.
- `monthly_free_grant_mc` / `rate_limit_per_min` — per-account overrides; NULL
  falls back to the global default (resolved in code, which can read config;
  migrations cannot).
- `free_grant_period` — idempotency marker for the lazy monthly top-up.

### `credit_ledger` — new append-only table

```sql
CREATE TABLE credit_ledger (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),  -- pgcrypto (migration 0001); lets every insert (migration seed + runtime settle) omit id

    account_id      UUID        NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    delta_mc        BIGINT      NOT NULL,          -- signed; +grant/+purchase, −refund/−adjustment
    reason          TEXT        NOT NULL CHECK (reason IN
                      ('free_grant','purchase','admin_grant','refund','adjustment','migration_seed')),
    external_ref    TEXT,                          -- e.g. Dodo payment id, admin user id
    balance_after_mc BIGINT     NOT NULL,          -- snapshot for audit/reconciliation
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    metadata        JSONB
);
CREATE INDEX credit_ledger_account_created_idx ON credit_ledger (account_id, created_at DESC);
```

Rows are written **only** for credits-in and adjustments — never per
consumption. Per-invocation detail already lives in `relay_invocations` (all
four surfaces log there). Invariant:
`credit_balance_mc == Σ(credit_ledger.delta_mc) − Σ(consumption)`.

### Dropped (in the contract migration)

`plans` (table), `accounts.plan_id` (column + FK), `usage_counters` (table) —
all used only by the quota path being replaced (`usage::summary` reads
`relay_invocations`, not `usage_counters`; verified).

## Lifecycle

### Lazy free top-up (rollover)

No cron (single-instance; only active accounts need it). On the enforce credit
check, in one transaction guarded on the marker:

```
current := date_trunc('month', now() AT TIME ZONE 'UTC')::date
if free_grant_period IS NULL OR free_grant_period < current:
    months := LEAST(12, month_diff(COALESCE(free_grant_period, current_minus_1mo), current))
    grant  := months * resolve_monthly_free_grant_mc(account)   -- COALESCE(col, config default)
    -- Single statement: the ledger row is written ONLY if the guarded UPDATE hit
    -- a row, so a grantor that loses the race writes neither balance nor ledger.
    WITH applied AS (
      UPDATE accounts
         SET credit_balance_mc = credit_balance_mc + grant,
             free_grant_period = current
       WHERE id = $acct AND free_grant_period IS NOT DISTINCT FROM $observed_period
      RETURNING credit_balance_mc
    )
    INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc, …)
    SELECT $acct, grant, 'free_grant', credit_balance_mc, … FROM applied;
```

- Rollover: grants are additive; unused free accumulates.
- Catch-up capped at 12 months so a long-dormant account can't accrue an
  unbounded windfall.
- Idempotent + atomic: the guarded `WHERE free_grant_period = observed` UPDATE
  and the ledger INSERT are one CTE, so a grantor that loses the race writes
  neither the balance bump nor a (spurious) ledger row — preserving the ledger
  invariant.
- Brand-new accounts (post-migration) have `credit_balance_mc = 0`,
  `free_grant_period = NULL`; their first enforce applies the current month's
  grant before the balance check, so the first invocation is covered. (Seeding
  balance at account-creation time is a Phase 2 nicety, not required here.)

### Consume

In the same row-write transaction as the `relay_invocations` insert (where
`quota::increment` is today), at each of the four surfaces:

```sql
UPDATE accounts SET credit_balance_mc = credit_balance_mc - $cost_mc WHERE id = $acct;
```

Unconditional decrement, matching today's check-then-increment **best-effort**
semantics: the balance was already gated in `enforce` before dispatch; a rare
concurrent race can dip the balance slightly negative, which the next enforce
blocks. This mirrors the rate-limiter's fail-open philosophy and never fails an
already-dispatched invocation. The gap is documented, not closed (single
instance, low per-account concurrency).

## Enforcement integration

`limits::enforce` keeps its shape and stays un-bypassable across all four
surfaces:

1. **Rate check** (Redis) — unchanged.
2. **Credit check** (replaces the quota check): apply any due free grant, then
   `credit_balance_mc >= cost_mc`.

Module rework (relay `src/limits/`):
- `quota.rs` → `credits.rs`: `current_month`/`increment` → `settle_free_grant`,
  `balance`, `consume(db, account, cost_mc)`.
- `mod.rs`: `resolve_plan`/`PlanLimits` → `resolve_limits`/`AccountLimits`
  (reads the per-account columns, COALESCE with config defaults; no `plans`
  join). `LimitOutcome::QuotaExceeded` → `InsufficientCredits`.
- `enforce()` shadow logic unchanged (`LIMITS_ENFORCE`): over-limit still logs
  `limit.would_block` and allows.
- Consume call sites reworked from `quota::increment` → `credits::consume`:
  `forwarder::persist_invocation`, `inbox_bridge::park`, `invoke_trusted`,
  `invoke_public`, `mcp::invoke`.

Error codes (rename from quota → credits) — **per-surface mapping is unchanged, only the variant/code/message rename**:
- **A2A:** `DenyReason::QuotaExceeded` → `InsufficientCredits`: keep jsonrpc
  `-32008` (so the existing `-32007 | -32008 => TOO_MANY_REQUESTS` arm in the real
  `jsonrpc_to_http` needs no change — avoids the historical wrong-match footgun),
  data code `chk.limit.credits`, message "insufficient credits".
- **`/v1/invoke`:** `ApiError::QuotaExceeded` → `InsufficientCredits`: 429, code
  `account_credits_exhausted`.
- **MCP:** unchanged mapping — `mcp.rs` returns a JSON-RPC `ERR_INVALID_REQUEST`
  (a client-side "back off" error, **not** a 429) carrying the new "insufficient
  credits" message. Only the message/variant changes here.
- `RateLimited` unchanged.

## Config (env, per-account overridable)

Added to `chakramcp_shared::config::SharedConfig`, read by the relay:

| Env | Default | Meaning |
|----|----|----|
| `HOSTING_MODE` | `self_hosted` | `managed` \| `self_hosted`. Phase 1 only stores it; it gates purchasing (P4) so a self-host never exposes a buy flow. |
| `CREDITS_DEFAULT_MONTHLY_FREE_MC` | `100000` (100 credits) | Monthly free grant when `accounts.monthly_free_grant_mc` is NULL. At 0.1/invocation = **1,000 invocations/mo, = today's free tier.** |
| `CREDITS_COST_PER_INVOCATION_MC` | `100` (0.1 credit) | Flat cost per invocation. |
| `LIMITS_DEFAULT_RATE_PER_MIN` | `60` | Rate limit when `accounts.rate_limit_per_min` is NULL. |

**Coupling to watch:** the expand migration seeds balances with the *literal*
`100000` (migrations can't read env). It must match
`CREDITS_DEFAULT_MONTHLY_FREE_MC`. Changing the env default later affects only
*future* grants for NULL-override accounts, not already-seeded balances.

## Migration & cutover — expand/contract

Enforcement is live and CD runs migrations **before** restarting the relay, so a
single destructive migration would leave the still-running old relay querying a
dropped `usage_counters`. Split into two migrations shipped in two deploys:

**`0033_credits_expand.sql` (deploy 1, with the credits code):**
1. `ALTER TABLE accounts ADD` the four new columns.
2. `CREATE TABLE credit_ledger` + its index. (Must precede any ledger INSERT.)
3. Backfill `rate_limit_per_min` from each account's current plan (via
   `plan_id` join) — preserves existing rate tiers.
4. Seed `credit_balance_mc = 100000`, `free_grant_period = current month`, and
   one `migration_seed` `credit_ledger` row per existing account.
- Keeps `plans`/`plan_id`/`usage_counters` intact, so the old relay serving
  during the migrate window still works. The new relay ignores them.

**`0034_credits_contract.sql` (deploy 2, after the credits code is live):**
5. `ALTER TABLE accounts DROP COLUMN plan_id;` then `DROP TABLE plans;`
   `DROP TABLE usage_counters;`
- Safe because no live code references them any more.

Forward-only (drops tables); rollback = restore from backup. Both deploys are
ordinary `backend/**` changes → CD builds + migrates + restarts.

### Rollout

Deploy 1 ships the credits mechanism while enforcement stays on. Optional
safety bake: flip `LIMITS_ENFORCE=false` briefly post-deploy 1 to confirm credit
accounting via `limit.would_block` logs, then re-enable — before deploy 2.

## Testing

- **`credits`:** atomic decrement; insufficient balance → block; lazy grant
  (rollover, 12-month cap, idempotent under concurrent settle); `resolve_limits`
  with and without per-account overrides; config-default resolution.
- **`enforce`:** shadow vs enforce with credits at a representative surface;
  `limit.would_block` shape.
- **Four surfaces:** port the existing quota tests to credits (A2A push/pull,
  `/v1/invoke` trusted/public, MCP) — assert balance decremented by `cost_mc`, and
  exhausted-account denial per surface: A2A + `/v1/invoke` → **429**
  (`account_credits_exhausted`); MCP → **JSON-RPC `ERR_INVALID_REQUEST`** with the
  "insufficient credits" message (not 429).
- **Migration:** expand backfill correctness (balance/ledger/rate limit seeded);
  expand keeps old tables; contract drops them; a fresh DB migrates cleanly end
  to end.
- **Live Redis** rate path stays green (reuse the `backend-ci` Redis service).

## Later phases

All phases build on this one substrate; mode differences are noted per phase.

- **P2 — Owner visibility (both modes):** balance, consumption, effective
  limits, grant history on `/app/usage` + `/app/account` (a read endpoint over
  `credit_balance_mc` + `credit_ledger` + `relay_invocations`). UI wording
  adapts to `HOSTING_MODE` ("credits" vs "allowance").
- **P3 — Admin management (both modes):** grant/adjust an account's balance, set
  its monthly grant + rate limit, observe usage; admin API + UI; every change a
  `credit_ledger` row (audited). In managed this is the superadmin (operator);
  in self-hosted it's the instance's own admin.
- **P4 — Purchasing (managed-only, Dodo Payments):** gated by
  `HOSTING_MODE=managed`. Checkout + signed webhooks → idempotent `purchase`
  ledger rows + balance top-up; refunds → `refund` rows; recurring auto-renewal
  as a Dodo subscription. `DODOPAYMENT_API_KEY` already in `.env.local`. Never
  exposed on a self-host.
- **P5 — Request-increase + alerts (both modes):** a user raises a
  quota-increase request to their admin (the primary top-up path on a
  self-host); usage alerts fire to the admin (and user) as balance nears
  exhaustion. Approval → an `admin_grant` ledger row (reuses P3).
- **P6 — Agent signaling:** 429 `Retry-After` + balance/limit headers so calling
  agents back off gracefully.
- **Cost weighting (ongoing):** promote `cost_mc()` from a flat value to a
  per-action-class table.

## Open items (resolved defaults, flag if you disagree)

- Monthly free grant default **100 credits** (preserves today's 1,000
  invocations/mo). Cost/invocation **0.1 credit**. Default rate **60/min**.
- Free-grant catch-up capped at **12 months**.
- `usage_counters` dropped outright (not kept for history — `relay_invocations`
  already holds per-invocation detail).
