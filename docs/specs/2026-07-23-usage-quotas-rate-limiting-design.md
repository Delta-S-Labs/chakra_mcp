# Usage quotas & rate limiting

**Status:** design (drafted 2026-07-23)
**Brainstormed via:** `/brainstorming`
**Depends on:** the relay policy gate (`backend/relay/src/policy/`), `relay_invocations` (migration 0024), the usage rollups (`backend/app/src/handlers/usage.rs`).

## Problem

Every A2A invocation flows through the relay unbounded. A buggy or hostile agent can hammer the relay or a target as fast as it can loop, and there is no ceiling on how much any one account may consume over time. Two protections are missing:

1. **Rate limiting** — velocity caps, so a runaway caller can't flood the relay/target in a tight loop.
2. **Quotas** — total-usage caps per account per period, as the enforcement substrate a tiered product needs.

There is no plan/tier concept in the schema today (`accounts` has `slug`, `account_type`, `owner_user_id`, `verified_at`, `tombstoned_at` — no notion of a limit), so quotas require inventing "a limit that attaches to an account." This spec introduces named plans and wires both protections into the existing relay decision gate.

## Goals

- **Named plans** (`free` / `pro` / `enterprise`) carrying a per-minute rate limit and a monthly invocation quota; every account has exactly one plan (default `free`).
- **Rate limiting** at the relay gate: per-account, fixed 60-second window, enforced via Redis.
- **Quotas** at the relay gate: per-account monthly invocation cap, tracked durably in Postgres.
- **Shadow-mode-first rollout:** a global flag ships enforcement in observe-only mode — every over-limit call is logged/counted but still authorized — so real traffic can be validated before anything is blocked.
- **Fail-open on the rate path:** if Redis is unreachable, allow the call and alert. A rate-limiter outage must not take the relay down.
- Both limits governed **per account** (the plan unit); the invocation counts against the **caller's** account.

## Non-goals

- **Billing / payment / self-serve plan changes.** No Stripe, no checkout. Plans are seeded; assignment is admin-only (a small endpoint) or direct SQL for v1.
- **Rate-limiting or quota-ing control-plane actions** (friendship proposals, grants, agent registrations, capability publishes). Cheap ops; deferred.
- **Per-credential / per-agent / per-pair granularity.** v1 is per-account only. Finer isolation is a later refinement behind the same trait.
- **Token-bucket / sliding-window smoothing.** v1 uses a fixed 60-second window; smoothing is a later refinement.
- **A usage-page quota indicator and admin plan-management UI.** Deferred to a v1.1 surfacing PR (see Rollout PR 4).
- **Multi-instance correctness beyond Redis.** The quota path is Postgres (already shared); the rate path is Redis (already shared). Both are multi-instance-safe by construction, but we are not adding relay replicas as part of this work.

## Approach

Enforcement lives at the **relay policy evaluator** (`backend/relay/src/policy/evaluator.rs`), the single gate every A2A call already runs through. It returns `Decision::Authorized | Denied(DenyReason)`; we add two deny reasons and two checks that run **after** authorization (friendship/grant) passes and **before** the forward.

Counting subject for both limits is the **caller's account** — resolved from the caller agent the gate has already validated.

### Data model (Rollout PR 1 — migrations only)

`backend/migrations/0032_plans_and_usage_limits.sql`:

```sql
CREATE TABLE IF NOT EXISTS plans (
    id                       UUID PRIMARY KEY,
    name                     TEXT NOT NULL UNIQUE,          -- 'free' | 'pro' | 'enterprise'
    rate_limit_per_min       INTEGER NOT NULL,             -- calls/min per account
    monthly_invocation_quota BIGINT,                       -- NULL = unlimited
    is_default               BOOLEAN NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- exactly one default plan
CREATE UNIQUE INDEX IF NOT EXISTS plans_one_default
    ON plans (is_default) WHERE is_default;

-- seed (placeholder numbers — tune before enforcing)
INSERT INTO plans (id, name, rate_limit_per_min, monthly_invocation_quota, is_default) VALUES
    (gen_random_uuid(), 'free',       60,   1000,  TRUE),
    (gen_random_uuid(), 'pro',        600,  50000, FALSE),
    (gen_random_uuid(), 'enterprise', 6000, NULL,  FALSE)
ON CONFLICT (name) DO NOTHING;

-- attach accounts to a plan, defaulting existing + new rows to free
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS plan_id UUID REFERENCES plans(id);
UPDATE accounts SET plan_id = (SELECT id FROM plans WHERE is_default) WHERE plan_id IS NULL;
ALTER TABLE accounts ALTER COLUMN plan_id SET NOT NULL;

-- durable monthly quota ledger
CREATE TABLE IF NOT EXISTS usage_counters (
    account_id   UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    period_start DATE NOT NULL,                            -- date_trunc('month', now())
    invocations  BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, period_start)
);
```

Migration-only; changes no queries, needs no `.sqlx` regen. CD applies it before the PR-3 binary deploy.

### Limiter module (Rollout PR 2 — infra + module, not yet wired)

New `backend/relay/src/limits/` with a `RateLimiter` trait so the check is mockable in tests and a no-op when Redis is absent:

```rust
pub enum RateOutcome { Allowed, Limited }

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Fixed 60s window keyed by (account, current minute).
    async fn check(&self, account: Uuid, per_min: i32) -> RateOutcome;
}
```

- **`RedisRateLimiter`** — `INCR rl:{account}:{unix_minute}`, `EXPIRE 60` on first hit, compare to `per_min`. On any Redis error → return `Allowed` and emit a `warn!` + a `ratelimit_redis_error` metric (fail-open). Pool via `deadpool-redis`.
- **`NoopRateLimiter`** — always `Allowed`; used when `REDIS_URL` is unset (local dev, tests).
- Quota lives in `quota.rs` (plain sqlx against `usage_counters`), no trait needed — Postgres is always present.

Config: `SharedConfig` gains `redis_url: Option<String>`; relay state holds `Arc<dyn RateLimiter>` (Redis impl if `redis_url` is set, else Noop). `docker-compose.prod.yml` gains a `redis:` (alpine) service on the internal network with a healthcheck; the relay `depends_on` it.

### Enforcement wiring (Rollout PR 3)

Two new `DenyReason` variants in `backend/relay/src/policy/decision.rs` (existing codes stop at `-32005`):

| Variant | JSON-RPC code | Slug |
|---|---|---|
| `RateLimited` | `-32006` | `chk.limit.rate` |
| `QuotaExceeded` | `-32007` | `chk.limit.quota` |

In `evaluator.rs`, after the authz branches, before the forward:

1. Resolve caller account → its plan (`rate_limit_per_min`, `monthly_invocation_quota`). One indexed join; cache per-request.
2. **Rate check:** `limiter.check(account, plan.rate_limit_per_min)`. `Limited` → `RateLimited`.
3. **Quota check:** read `usage_counters` for `(account, date_trunc('month', now()))`. If `monthly_invocation_quota` is non-NULL and `invocations >= quota` → `QuotaExceeded`.

**Shadow mode:** global `LIMITS_ENFORCE` env bool, **default false**. When a check would deny and `LIMITS_ENFORCE` is false, emit a structured `info!(event = "limit.would_block", kind, account, plan)` log + a metric counter, and return `Authorized` anyway. When true, return the `Denied`.

**Quota increment:** on an authorized forward, `INSERT INTO usage_counters (account_id, period_start, invocations) VALUES ($1, date_trunc('month', now()), 1) ON CONFLICT (account_id, period_start) DO UPDATE SET invocations = usage_counters.invocations + 1`, executed inside the existing `record_terminal` transaction (`backend/relay/src/handlers/invoke.rs:238`) so the counter never drifts from the `relay_invocations` ledger. Regenerate the `.sqlx` cache in this PR.

## What counts

- **Only invocations** written to `relay_invocations` via `invoke_trusted` and `invoke_public` (`backend/relay/src/handlers/invoke.rs`).
- Counted against the **caller's account** (the initiator), resolved from the validated caller agent.
- **Inbox pulls do not count** — the invocation was already counted when the caller enqueued it; the target claiming work is not a second event.
- **Control-plane actions do not count** (friendships/grants/registrations/capability publishes).

## Reset semantics

- **Rate:** fixed 60-second window (`rl:{account}:{unix_minute}`, `EXPIRE 60`). A brief double-burst at a window boundary is acceptable for v1; token-bucket smoothing is a later refinement.
- **Quota:** calendar month, UTC — `period_start = date_trunc('month', now())`. Rollover is automatic (a new month is a fresh `usage_counters` row starting at 0); no cron.

## Decisions

### D1 — Redis for rate, Postgres for quota (Approach C)
Rate limiting needs a fast, atomic, shared counter; Redis is the standard fit and is multi-instance-safe if the relay ever scales. Quota needs durability and already has a home in Postgres. Rejected: all-Postgres (a hot-path DB write per call) and in-memory (per-instance drift if scaled). Cost: a new Redis dependency in prod infra, mitigated by making it optional + fail-open.

### D2 — Named plans now, not a flat default
Chosen over "one config default + per-account override." Plans are the monetization substrate; modeling them now avoids a migration later. A plan is just a named bundle of the same limit columns, so the extra cost over the flat approach is one small table + an FK.

### D3 — Shadow-mode-first, global flag
`LIMITS_ENFORCE=false` by default: build real enforcement, observe would-blocks against live traffic, then flip on. Global (not per-plan) for v1 simplicity; per-plan staged rollout is a later refinement. This is the safest way to ship a gate that can reject legitimate traffic.

### D4 — Fail-open on the rate path
Redis unreachable → allow + alert. Rate limiting is a protection, not a correctness invariant; availability beats enforcement. Quota is Postgres (already a hard dependency), so it introduces no new failure mode.

### D5 — Per-account subject for both
The plan governs the account, so both limits key on the account. A single runaway credential can consume its account's budget; finer per-credential isolation is deferred behind the same trait. Consistent and simple.

### D6 — Check-before, increment-after for quota
The quota is read before the forward and the counter incremented after (in `record_terminal`). Under extreme concurrency a handful of calls can slip over the monthly cap; acceptable for a period quota. Strict atomic increment-and-check is a later refinement if needed.

## Security note

- **Plan is server-side**, derived from the caller's account; a caller cannot request or spoof a higher tier.
- Quota counts are per-account and visible only to account members (same posture as the existing usage rollups).
- Deny reasons return stable slugs (`chk.limit.rate` / `chk.limit.quota`) and standard JSON-RPC codes, so clients can distinguish "slow down" from "you're out of quota" without leaking another account's numbers.

## Testing

- **Quota** (`sqlx::test`): at-limit → `QuotaExceeded`; under-limit → authorized; NULL quota (enterprise) → unlimited; month rollover → fresh counter starts at 0; increment lands inside the `record_terminal` transaction (rolls back with it).
- **Rate**: via the `RateLimiter` trait's in-memory test impl (no real Redis in CI) — over-window → `RateLimited`; `NoopRateLimiter` → always allowed (fail-open / Redis-absent path).
- **Shadow**: `LIMITS_ENFORCE=false` → over-limit calls are authorized but a `limit.would_block` event is recorded; `=true` → over-limit calls are denied.
- **Plan resolution / adversarial**: account with no explicit plan resolves to `free`; a caller cannot cause a different plan to be applied than its account's.
- **Regression**: existing policy-gate tests (auth, friendship, grant, unreachable) stay green; the new checks are strictly additive and skip cleanly when under limit.

CI note: the `sqlx::test` suite must not require Redis — rate-limit logic is exercised through the trait's in-memory impl. The `RedisRateLimiter` gets a lighter, optionally-gated integration test.

## Rollout

1. **PR 1 — migrations only.** `plans` + seed, `accounts.plan_id` (default free + backfill + NOT NULL), `usage_counters`. Applies via `task migrate` before any binary swap. Verify with `\d accounts` / `\d usage_counters`.
2. **PR 2 — infra + limiter module.** Redis service in `docker-compose.prod.yml`, `redis_url` config, `deadpool-redis` dep, the `limits/` module (`RateLimiter` trait + Redis + Noop impls, `quota.rs`), unit tests. Not wired to the gate yet.
3. **PR 3 — enforcement integration.** Two `DenyReason` variants, the two checks in `evaluator.rs`, quota increment in `record_terminal`, `LIMITS_ENFORCE` flag (default shadow), `.sqlx` regen, full test suite. Ship in shadow mode; watch `limit.would_block` metrics before flipping `LIMITS_ENFORCE=true`.
4. *(v1.1, separate)* — usage-page "used X / Y this month" indicator + an admin endpoint to set an account's plan.

## Out of scope (logged as backlog)

- Token-bucket / sliding-window rate smoothing.
- Per-credential / per-agent / per-pair limits.
- Quota/rate on control-plane actions.
- Self-serve billing and plan upgrades.
- Per-plan staged enforcement (vs the single global `LIMITS_ENFORCE`).

## Why this matters

Today the relay trusts every authorized caller to behave. Rate limiting turns "please don't loop" into an enforced ceiling, and quotas turn "usage" into something a plan can actually bound — the substrate any paid tier needs. Shipping both behind a shadow flag means we get the enforcement machinery in place and watch exactly who it would affect, before a single legitimate call is ever rejected.
