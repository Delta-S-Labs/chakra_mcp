# Usage quotas & rate limiting

**Status:** design (drafted 2026-07-23, revised after spec-review iteration 1)
**Brainstormed via:** `/brainstorming`
**Depends on:** the relay policy gate (`backend/relay/src/policy/`), `relay_invocations`, the invocation write paths (`forwarder.rs`, `inbox_bridge.rs`, `handlers/invoke.rs`, `handlers/mcp.rs`), the usage rollups (`backend/app/src/handlers/usage.rs`).

## Problem

Every A2A invocation flows through the relay unbounded. A buggy or hostile agent can hammer the relay or a target as fast as it can loop, and there is no ceiling on how much any one account may consume over time. Two protections are missing:

1. **Rate limiting** — velocity caps, so a runaway caller can't flood the relay/target in a tight loop.
2. **Quotas** — total-usage caps per account per period, as the enforcement substrate a tiered product needs.

There is no plan/tier concept in the schema today (`accounts` has `slug`, `account_type`, `owner_user_id`, `verified_at`, `tombstoned_at` — no notion of a limit; confirmed against migrations `0001`/`0009`), so quotas require inventing "a limit that attaches to an account." This spec introduces named plans and wires both protections into every invocation path.

## Goals

- **Named plans** (`free` / `pro` / `enterprise`) carrying a per-minute rate limit and a monthly invocation quota; every account has exactly one plan (default `free`).
- **Rate limiting**, per-account, fixed 60-second window, enforced via Redis.
- **Quotas**, per-account monthly invocation cap, tracked durably in Postgres.
- **Un-bypassable:** both limits enforced at *every* production invocation surface (A2A push, A2A pull, legacy `/v1/invoke`, MCP proxy) through one shared primitive — not on a single path a caller could route around.
- **Shadow-mode-first rollout:** a global flag ships enforcement in observe-only mode — every over-limit call is logged/counted but still allowed — so real traffic can be validated before anything is blocked.
- **Fail-open on the rate path:** if Redis is unreachable, allow the call and alert. A rate-limiter outage must not take the relay down.
- Governed **per account** (the plan unit); the invocation counts against the **caller's** (initiator's) account.

## Non-goals

- **Billing / payment / self-serve plan changes.** No Stripe, no checkout. Plans are seeded; assignment is admin-only (a small endpoint) or direct SQL for v1.
- **Rate-limiting or quota-ing control-plane actions** (friendship proposals, grants, agent registrations, capability publishes). Cheap ops; deferred.
- **Per-credential / per-agent / per-pair granularity.** v1 is per-account only. Finer isolation is a later refinement behind the same primitive.
- **Token-bucket / sliding-window smoothing.** v1 uses a fixed 60-second window; smoothing is a later refinement.
- **A usage-page quota indicator and admin plan-management UI.** Deferred to a v1.1 surfacing PR (Rollout PR 4).
- **Per-plan staged enforcement.** v1 has a single global enforce flag; per-plan flags are a later refinement.

## The four invocation surfaces (established by review)

Production code writes an invocation row to `relay_invocations` at four sites, under three different authorization mechanisms. All four must be governed or the quota is trivially evadable:

| # | Surface | Write site | Authorization | Caller-account source |
|---|---|---|---|---|
| 1 | A2A push | `forwarder.rs::forward_push` (~`:226`) | `policy::evaluate()` | `Authorized.caller_account_id` |
| 2 | A2A pull | `inbox_bridge.rs::park` (~`:128`) | `policy::evaluate()` | `Authorized.caller_account_id` |
| 3 | Legacy `/v1/invoke` | `handlers/invoke.rs::invoke_trusted` (~`:517`) / `invoke_public` (~`:657`) | own inline checks | grantee agent's account (from the resolved grant/agent row) |
| 4 | MCP proxy | `handlers/mcp.rs` invoke (~`:939`) | own grant + `user_is_member` check | grantee/caller account from the grant row |

`relay_invocations` has no `account_id` column; the caller's account is derived via `grantee_agent_id → agents.account_id` (or `invoked_by_user_id`). On the A2A path the account is already in hand as `Authorized.caller_account_id`, which is the cleanest source at check time. The INSERTs at `grants.rs:626`, `reviews.rs:848`, and `invoke.rs:2258` are `#[cfg(test)]` seed helpers and are **not** counted. `record_terminal` (`invoke.rs:238`) is **not** a counting site — it runs a single non-transactional `execute()` only on *rejected* legacy rows.

## Approach

A single shared primitive, applied identically at all four surfaces:

- **`limits::check(db, limiter, caller_account_id) → LimitOutcome`** — resolves the account's plan, runs the rate check (Redis) and the quota check (Postgres), returns `Allowed | RateLimited | QuotaExceeded`.
- **`limits::increment(&mut tx, caller_account_id)`** — bumps the monthly counter, called inside the same transaction that writes the `relay_invocations` row.

**Counting rule: one increment per `relay_invocations` row-write, regardless of the call's outcome** (succeeded / failed / timeout). This keeps counting uniform across surfaces even though `forward_push` writes its row *after* the upstream call with a terminal status while the other three write a `pending` row at accept time — do **not** gate the push increment on `status == 'succeeded'`.

Each surface calls `check()` before doing work and maps a non-`Allowed` outcome to its own transport error, and calls `increment()` in the row-write transaction. This co-locates check and count on every path and keeps the counter consistent with the ledger. All four write sites (`park` `inbox_bridge.rs:145`, `forward_push`'s `persist_invocation` `forwarder.rs:249`, the legacy enqueues `invoke.rs:517`/`:657`, the MCP INSERT `mcp.rs:~939`) are currently bare `execute`s, so each gains a `BEGIN`/`COMMIT` to wrap the row-write + increment; for push this stays short because the row is written after the HTTP round-trip (no network held inside the transaction).

### Data model (Rollout PR 1 — migrations only)

`backend/migrations/0032_plans_and_usage_limits.sql` (0032 confirmed as the next free number after `0031_device_visibility_hint_org.sql`):

```sql
CREATE TABLE IF NOT EXISTS plans (
    id                       UUID PRIMARY KEY,
    name                     TEXT NOT NULL UNIQUE,          -- 'free' | 'pro' | 'enterprise'
    rate_limit_per_min       INTEGER NOT NULL,             -- calls/min per account
    monthly_invocation_quota BIGINT,                       -- NULL = unlimited
    is_default               BOOLEAN NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS plans_one_default
    ON plans (is_default) WHERE is_default;

-- seed (placeholder numbers — tune before enforcing)
INSERT INTO plans (id, name, rate_limit_per_min, monthly_invocation_quota, is_default) VALUES
    (gen_random_uuid(), 'free',       60,   1000,  TRUE),
    (gen_random_uuid(), 'pro',        600,  50000, FALSE),
    (gen_random_uuid(), 'enterprise', 6000, NULL,  FALSE)
ON CONFLICT (name) DO NOTHING;

ALTER TABLE accounts ADD COLUMN IF NOT EXISTS plan_id UUID REFERENCES plans(id);
UPDATE accounts SET plan_id = (SELECT id FROM plans WHERE is_default) WHERE plan_id IS NULL;
ALTER TABLE accounts ALTER COLUMN plan_id SET NOT NULL;

CREATE TABLE IF NOT EXISTS usage_counters (
    account_id   UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    period_start DATE NOT NULL,                            -- date_trunc('month', now() AT TIME ZONE 'UTC')
    invocations  BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, period_start)
);
```

Migration-only; changes no queries, needs no `.sqlx` regen. CD applies it before the PR-3 binary deploy.

### Limiter module (Rollout PR 2 — infra + module, not yet wired)

New `backend/relay/src/limits/` with a `RateLimiter` trait so the rate check is mockable in tests and a no-op when Redis is absent:

```rust
pub enum RateOutcome { Allowed, Limited }

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Fixed 60s window keyed by (account, current minute).
    async fn check(&self, account: Uuid, per_min: i32) -> RateOutcome;
}
```

- **`RedisRateLimiter`** — `INCR rl:{account}:{unix_minute}`, `EXPIRE 60` on first hit, compare to `per_min`. On any Redis error → `Allowed` + `warn!` + a `ratelimit_redis_error` metric (fail-open). Pool via `deadpool-redis`.
- **`NoopRateLimiter`** — always `Allowed`; used when `REDIS_URL` is unset (local dev, tests).
- Quota lives in `quota.rs` (plain sqlx against `usage_counters`); no trait — Postgres is always present.
- The module also exposes the surface-agnostic `check()` / `increment()` primitives above and a `LimitOutcome` enum.

Config: `SharedConfig` gains `redis_url: Option<String>`; relay state holds `Arc<dyn RateLimiter>` (Redis impl if set, else Noop). `docker-compose.prod.yml` gains a `redis:` (alpine) service on the internal network with a healthcheck; the relay `depends_on` it.

### Enforcement wiring (Rollout PR 3)

**Two new `DenyReason` variants** in `backend/relay/src/policy/decision.rs`. Existing codes in use are `-32000/-32001/-32002/-32003/-32005/-32006`; `-32006` is already the target-not-found family, so the new reasons take the free codes:

| Variant | JSON-RPC code | `data.code` slug | HTTP |
|---|---|---|---|
| `RateLimited` | `-32007` | `chk.limit.rate` | 429 |
| `QuotaExceeded` | `-32008` | `chk.limit.quota` | 429 |

`a2a.rs::jsonrpc_to_http()` currently maps `-32003 → 403`, `-32006 → 404`, and **defaults unknown codes to 200**. Add `-32007 | -32008 => StatusCode::TOO_MANY_REQUESTS`, so an A2A limit denial surfaces as 429 (matching the legacy public-quota path's 429 at `invoke.rs:635`).

**Per-surface wiring** (all call the shared primitive):

- **A2A (surfaces 1 & 2):** in `a2a.rs::handle_send_message`, after `evaluate()` returns `Authorized(authz)` and before `park` / `forward_push`, call `limits::check(db, limiter, authz.caller_account_id)`; on `RateLimited` / `QuotaExceeded` return `deny_response(DenyReason::…)`. `increment()` runs inside the `park` / `forward_push` transaction that writes the row. (Equivalently the check can be the final branch inside `evaluate()`; keeping it in `handle_send_message` leaves `evaluate()` as pure authz. Either way it shares the same `limits::check`.)
- **Legacy `/v1/invoke` (surface 3):** call `limits::check(…)` once the grant/agent is resolved (the caller account isn't known before then — mirror the MCP placement, not literally the top of the fn); on a hit, return HTTP 429 with a quota payload that carries a **distinct `data.code`** (e.g. `account_monthly_quota_exhausted`) so clients can tell it apart from the existing per-capability `monthly_quota_exhausted` at `invoke.rs:635`. `increment()` joins the enqueue INSERT (`:517` / `:657`) in the same transaction.
- **MCP proxy (surface 4):** call `limits::check(…)` with the grantee/caller account after the grant + membership check, before the `relay_invocations` INSERT at `mcp.rs:~939`; map a hit to an MCP tool error. `increment()` in the same transaction as that INSERT.

**Shadow mode:** global `LIMITS_ENFORCE` env bool, **default false**. When `check()` would return a non-`Allowed` outcome and `LIMITS_ENFORCE` is false, each caller emits a structured `info!(event = "limit.would_block", kind, account, surface)` log + a metric counter and proceeds as `Allowed`. When true, the caller returns the surface's error. `increment()` still runs on every allowed-or-shadowed successful invocation, so the counter reflects real usage during observation.

**Quota check + increment SQL:**

```sql
-- check (read)
SELECT invocations FROM usage_counters
 WHERE account_id = $1 AND period_start = date_trunc('month', now() AT TIME ZONE 'UTC')::date;
-- increment (in the success-write txn)
INSERT INTO usage_counters (account_id, period_start, invocations)
VALUES ($1, date_trunc('month', now() AT TIME ZONE 'UTC')::date, 1)
ON CONFLICT (account_id, period_start) DO UPDATE
   SET invocations = usage_counters.invocations + 1;
```

Regenerate the `.sqlx` cache in this PR.

## Reset semantics

- **Rate:** fixed 60-second window (`rl:{account}:{unix_minute}`, `EXPIRE 60`). A brief double-burst at a window boundary is acceptable for v1; token-bucket smoothing is a later refinement.
- **Quota:** calendar month, explicitly UTC — `date_trunc('month', now() AT TIME ZONE 'UTC')`. Rollover is automatic (a new month is a fresh `usage_counters` row starting at 0); no cron.

## Decisions

### D1 — Redis for rate, Postgres for quota
Rate limiting needs a fast, atomic, shared counter; Redis fits and is multi-instance-safe if the relay ever scales. Quota needs durability and already has a home in Postgres. Rejected: all-Postgres (a hot-path DB write per call) and in-memory (per-instance drift if scaled). Cost: a new Redis dependency, mitigated by making it optional + fail-open.

### D2 — Named plans now, not a flat default
Chosen over "one config default + per-account override." Plans are the monetization substrate; modeling them now avoids a migration later. A plan is a named bundle of the same limit columns, so the extra cost is one small table + an FK.

### D3 — Shadow-mode-first, global flag
`LIMITS_ENFORCE=false` by default: build real enforcement, observe would-blocks against live traffic, then flip on. Global (not per-plan) for v1 simplicity. Safest way to ship a gate that can reject legitimate traffic.

### D4 — Fail-open on the rate path
Redis unreachable → allow + alert. Rate limiting is a protection, not a correctness invariant; availability beats enforcement. Quota is Postgres (already a hard dependency), so it adds no new failure mode.

### D5 — Per-account subject for both
The plan governs the account, so both limits key on the caller's account. A single runaway credential can consume its account's budget; finer per-credential isolation is deferred behind the same primitive.

### D6 — Shared primitive at all four surfaces (not one gate)
The policy gate (`evaluate()`) covers only the A2A path; legacy `/v1/invoke` and the MCP proxy authorize independently and would otherwise bypass the quota. So check + increment live in a shared `limits` module called at every surface, rather than as branches inside `evaluate()`. This is the correction from review iteration 1.

### D7 — Check-before, increment-after, per surface
Each surface reads the quota before doing work and increments the counter in the success-write transaction. Under extreme concurrency a handful of calls can slip over the monthly cap; acceptable for a period quota. Strict atomic increment-and-check is a later refinement.

### D8 — Composition with the existing public-invoke quota
Migration `0022` already enforces `public_monthly_quota_per_agent` — a per-capability cap on *public* invokes, returning 429 at `invoke.rs:635`. The new per-account quota is an **independent, additional** ceiling: a public invoke must pass both (the per-capability public cap *and* the caller-account cap). They read/write different stores (`relay_invocations` count vs `usage_counters`) and do not interfere. The two 429s carry distinct `data.code`s (`monthly_quota_exhausted` vs `account_monthly_quota_exhausted`) so a client can tell which ceiling it hit.

## Security note

- **Plan is server-side**, derived from the caller's account; a caller cannot request or spoof a higher tier.
- Quota counts are per-account and visible only to account members (same posture as the existing usage rollups).
- Deny reasons return stable slugs (`chk.limit.rate` / `chk.limit.quota`) and 429s, so clients can tell "slow down" from "out of quota" without leaking another account's numbers.

## Testing

- **Quota** (`sqlx::test`): at-limit → hit; under-limit → allowed; NULL quota (enterprise) → unlimited; month rollover → fresh counter at 0; increment lands inside the success-write transaction (rolls back with it).
- **Rate**: via the `RateLimiter` trait's in-memory test impl (no real Redis in CI) — over-window → `Limited`; `NoopRateLimiter` → always allowed (fail-open / Redis-absent path).
- **Per-surface**: a limit hit is mapped correctly on each surface — A2A → `DenyReason` + 429 via `jsonrpc_to_http`; `/v1/invoke` → 429; MCP → MCP tool error. Increment fires on each surface's success path.
- **Shadow**: `LIMITS_ENFORCE=false` → over-limit calls proceed but record `limit.would_block`; `=true` → they're rejected.
- **Composition**: a public invoke over the per-account cap is rejected even when under `public_monthly_quota_per_agent`, and vice-versa.
- **Plan resolution / adversarial**: no-plan account resolves to `free`; a caller cannot cause another plan to apply.
- **Regression**: existing policy-gate tests and the `legacy_v01_contract_tests` stay green; the checks are additive and skip cleanly under limit.

CI note: the `sqlx::test` suite must not require Redis — rate logic is exercised through the trait's in-memory impl. `RedisRateLimiter` gets a lighter, optionally-gated integration test.

## Rollout

1. **PR 1 — migrations only.** `plans` + seed, `accounts.plan_id` (default free + backfill + NOT NULL), `usage_counters`. Applies via `task migrate` before any binary swap.
2. **PR 2 — infra + limiter module.** Redis service in `docker-compose.prod.yml`, `redis_url` config, `deadpool-redis`, the `limits/` module (`RateLimiter` trait + Redis + Noop impls, `quota.rs`, the shared `check`/`increment`/`LimitOutcome`), unit tests. Not wired to any surface yet.
3. **PR 3 — enforcement integration.** Two `DenyReason` variants + `jsonrpc_to_http` 429 mapping; `limits::check`/`increment` wired into all four surfaces; `LIMITS_ENFORCE` flag (default shadow); `.sqlx` regen; full test suite. Ship in shadow mode; watch `limit.would_block` before flipping `LIMITS_ENFORCE=true`.
4. *(v1.1, separate)* — usage-page "used X / Y this month" indicator + an admin endpoint to set an account's plan.

## Out of scope (logged as backlog)

- Token-bucket / sliding-window rate smoothing.
- Per-credential / per-agent / per-pair limits.
- Quota/rate on control-plane actions.
- Self-serve billing and plan upgrades.
- Per-plan staged enforcement (vs the single global `LIMITS_ENFORCE`).

## Why this matters

Today the relay trusts every authorized caller to behave. Rate limiting turns "please don't loop" into an enforced ceiling, and quotas turn "usage" into something a plan can actually bound — the substrate any paid tier needs. Enforcing at every invocation surface makes the ceiling real rather than one route among several; shipping behind a shadow flag means we watch exactly who it would affect before a single legitimate call is rejected.
