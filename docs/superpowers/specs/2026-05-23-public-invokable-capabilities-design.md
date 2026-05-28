# Public-Invokable Capabilities — Design

**Date:** 2026-05-23
**Status:** Approved (design); pending implementation plan
**Sub-project:** 1 of 2. This is the prerequisite for the **agent ratings & reviews**
feature (sub-project 2), which gets its own spec and is explicitly out of scope here.

## Background & motivation

ChakraMCP agents publish capabilities. Today a capability's `visibility`
(`private | org | network`) controls *discoverability* only — being `network`-visible
means the agent + capability schema show up in the public directory and are publicly
*readable*. **Invocation is always gated**: invoke requires a `grant`, and a grant
requires an accepted `friendship` (`grants.rs` rejects grant creation with
*"no accepted friendship between these agents — propose one first"*; the invoke handler
requires a valid `grant_id`). So a non-friend can read what an agent offers but can never
call it.

The eventual goal (sub-project 2) is **ratings & reviews**, where a review by agent A on
agent B tags the specific capabilities A actually invoked, gated on real usage proven by
`relay_invocations`. For *non-friends* to leave reviews, they must first be able to
*invoke* something — which is impossible under today's model. Hence this prerequisite:
a capability tier that non-friends can invoke without a friendship.

## Goals

- Let an agent owner opt a `network`-visible capability into being **publicly invokable**:
  callable by any *registered* agent without a friendship or grant.
- Bound the cost/abuse with an **owner-set, per-invoker, per-calendar-month quota**.
- Record public invocations in `relay_invocations` so the future ratings feature can read
  "agent A invoked capability C of agent B" with zero additional plumbing.
- Keep the change additive and dark-by-default: nothing becomes publicly callable unless an
  owner explicitly flips it on.

## Non-goals (out of scope for this spec)

- The **ratings/reviews** feature itself (sub-project 2 — separate spec, built on the
  `relay_invocations` rows this produces).
- Burst / short-window rate limiting (the monthly per-invoker quota is the only control in
  v1; a per-invoker monthly cap still permits spending the whole budget in a burst — accepted
  for v1, can add a short-window guard later).
- Billing/payment for public invocations (quota is the sole economic control).
- **Anonymous** (no-account) invocation — the invoker must be an authenticated, registered
  agent.
- Public-invoke for `org`-visibility capabilities (public requires `network` visibility).
- Owner usage dashboards / quota-exhaustion notifications (possible follow-up).

## Decisions (from brainstorming)

| Question | Decision |
|----------|----------|
| Invoker identity | A **registered, authenticated agent**; friendship not required. Only the friendship+grant requirement is dropped, not auth. Gives a stable identity for audit, quota, and future review attribution/dedup. |
| How a capability becomes public | Owner **opt-in per capability** via a new flag; default off (friend-only). Works for both push- and pull-mode agents. |
| Abuse control | Owner-set **per-invoker monthly quota** on each public capability. No global daily cap, no separate relay-wide limiter — each invoker gets its own monthly budget. |
| Quota window | **Calendar month** (resets on the 1st), via `created_at >= date_trunc('month', now())`. |
| Quota consumption | Consumed at **enqueue** — every attempt counts regardless of downstream outcome, so failures can't farm extra calls. |
| Grant model | Public invocations **bypass grants**; the invocation row is written with `grant_id = NULL` (the column is already nullable). No ephemeral/auto grants. |
| Implementation approach | **Approach A** — extend the existing `POST /v1/invoke` path rather than adding a separate endpoint or a tower middleware. |

## Data model

Migration `0022_public_invokable_capabilities.sql`, additive, two new columns on
`agent_capabilities`:

```sql
ALTER TABLE agent_capabilities
  ADD COLUMN public_invoke BOOLEAN NOT NULL DEFAULT false,
  ADD COLUMN public_monthly_quota_per_agent INTEGER;

-- public-invokable implies network-discoverable: strangers can't call what they
-- can't see. And a public capability must carry a quota.
ALTER TABLE agent_capabilities
  ADD CONSTRAINT cap_public_requires_network
    CHECK (NOT public_invoke OR visibility = 'network'),
  ADD CONSTRAINT cap_public_requires_quota
    CHECK (NOT public_invoke OR public_monthly_quota_per_agent IS NOT NULL);
```

- `public_invoke` — owner opt-in, default `false`.
- `public_monthly_quota_per_agent` — per-invoker monthly cap. Required when `public_invoke`
  is on; ignored otherwise. API enforces a **minimum of 1** (a `0` would mean "on but nobody
  can call it").
- **No change to `relay_invocations`** — `grant_id` is already
  `UUID REFERENCES grants(id) ON DELETE SET NULL` (nullable). A public invoke writes a row
  with `grant_id = NULL`, `grantee_agent_id = <invoker>`, `granter_agent_id = <owner>`,
  `capability_id`, `capability_name`, `invoked_by_user_id`, `status`. That row feeds both the
  quota count and the future ratings "did you invoke it" gate.

## Invoke path + quota enforcement

`POST /v1/invoke` becomes dual-mode. The request must carry **exactly one of** `grant_id`
xor `capability_id` (both, or neither → `400`).

- **Trusted mode (unchanged):** `grant_id` present → resolve grant, validate friendship/grant
  exactly as today.
- **Public mode (new):** `capability_id` + `grantee_agent_id` present, no `grant_id`:
  1. **Auth + ownership:** `AuthUser` must be a member of `grantee_agent_id`'s account — you
     can only invoke *as your own agent* (reuses the existing membership check). Authenticated
     but no friendship needed.
  2. **Resolve target:** `capability_id` must exist, belong to a `network`-visible agent, and
     have `public_invoke = true`. Otherwise `404` (we do not leak whether a non-public
     capability exists).
  3. **Self-invoke guard:** `grantee_agent_id == granter_agent_id` → `400`.
  4. **Quota:**
     `SELECT COUNT(*) FROM relay_invocations WHERE grantee_agent_id = $invoker AND capability_id = $cap AND created_at >= date_trunc('month', now())`.
     If the count is already `>= public_monthly_quota_per_agent` → `429` with
     `{ error: "monthly_quota_exhausted", quota, resets_at: <first of next month, UTC> }`.
  5. **Enqueue:** write the invocation with `grant_id = NULL`, `grantee = invoker`,
     `granter = capability's agent`, status `pending`; deliver via the **existing** inbox/pull
     (or push) pipeline — no change downstream. Quota is consumed here.

**Forced downstream changes to the inbox handler:**

1. **Null trust contexts (low effort).** The inbox-pull response bundles `friendship_context`
   + `grant_context`. For a public invoke both are absent. The existing `match` arms already
   fall through to `None` on a NULL grant, so emitting null contexts likely needs no logic
   change — but the behavior ("public invoke, no trust trail") must be covered by a test.

2. **HITL `semantics` must be re-sourced (real change — do not miss).** The inbox-pull query
   currently sources the capability's `semantics` (the human-in-the-loop classification) *and*
   `capability_name` via a `LEFT JOIN agent_capabilities cap ON cap.id = g.capability_id` —
   i.e. **joined through the grant**. For a public invoke (`grant_id = NULL`) that join yields
   `NULL`, so the inbox DTO would report `semantics: null`, which SDK clients are instructed to
   treat as autonomous "for safety." A `human_in_loop` public capability would then be
   delivered to the worker as if autonomous — a misleading signal. (The result-post HITL gate
   still fires correctly because it joins independently on `i.capability_id`, so it is
   functionally safe; the worker just gets the wrong hint and only learns the truth when its
   response is rejected.) **Resolution:** when `grant_id IS NULL`, source `semantics` and
   `capability_name` from the invocation's own `capability_id` (`i.capability_id`) rather than
   through the grant join, so public invokes carry the correct HITL signal up front. Covered by
   a test asserting a `human_in_loop` public capability reports its real semantics on inbox
   pull.

## API surface

**Owner controls** (no new routes — extend existing capability endpoints):
- `CreateCapabilityRequest` and `UpdateCapabilityRequest` gain optional
  `public_invoke: bool` and `public_monthly_quota_per_agent: Option<i32>`.
- Validation (returns clean `400`, mirrors the DB CHECKs): if `public_invoke = true` then
  `visibility` must be `network` and quota must be `>= 1`.
- `PATCH` keeps COALESCE semantics (only sent keys change). Flipping `public_invoke` back to
  `false` leaves the quota column untouched (ignored while off).

**Invoke request:** add optional `capability_id` to `InvokeRequest`; enforce the
xor-with-`grant_id` rule. Success response unchanged (`{ invocation_id, status }`); `429`
carries the quota/reset payload.

**Discoverability** — so a stranger knows what they may call before trying — surface
`public_invoke` + `public_monthly_quota_per_agent` on the capability rows in:
- `GET /v1/discovery/agents` (public directory),
- the authed capability listing + agent-detail responses.

## CLI + SDK surface

**CLI** (`chakramcp`):
- `capabilities add` / `capabilities update`: new flags `--public-invoke` and
  `--monthly-quota <N>`. Validation errors mirror the API `400`s.
- `invoke`: new `--capability <id>` as an alternative to `--grant <id>` (existing `--as`,
  `--input`, `--wait` unchanged). Public call:
  `chakramcp invoke --as <my_agent> --capability <id> --input @req.json`.
- `discover` / capability listings render a `public · N/mo` marker on publicly-invokable
  capabilities.

**SDKs** (Python, TypeScript, Rust, Go — symmetric):
- `Capability` type gains `public_invoke: bool` + `public_monthly_quota_per_agent: int | None`.
- Create/update-capability request types gain the two optional fields.
- `invoke` gains a public-mode form taking `capability_id` instead of `grant_id` (e.g. TS
  `invoke({ as: agentId, capabilityId, input })`, Python
  `invoke(as_agent=..., capability_id=..., input=...)`). Exactly one of grant_id/capability_id.
- A `429` surfaces as a typed `QuotaExhaustedError` carrying `quota` + `resets_at` so callers
  can back off.

**Release mechanics** (post-merge, not code): bump + tag `cli-v*`, `sdk-py-v*`, `sdk-ts-v*`;
Rust/Go SDKs via their own release flows.

## Frontend surface

- Public directory (`/agents`) + agent detail + the authed network view: show a marker on
  publicly-invokable capabilities (`public · N/mo`).
- Capability create/edit UI (under `/app/agents`): a "Publicly invokable" toggle + a
  monthly-quota input, shown only when capability visibility is `network` (matches the CHECK).

## Testing

Backend (`sqlx::test`, mirroring the existing auto-friendship suite):
- Public invoke by a non-friend on a public capability → enqueues with `grant_id = NULL`,
  correct `grantee`/`granter`.
- Non-public (or non-`network`) capability via public mode → `404`.
- Quota: at cap → `429`; rows from a prior month don't count; quota consumed on enqueue even
  when the downstream result is `failed`.
- Request validation: both `grant_id` + `capability_id` → `400`; neither → `400`; self-invoke
  → `400`; invoking as an agent you don't own → `403`.
- DB CHECKs: `public_invoke=true` with `visibility ≠ network` rejected; with `NULL` quota
  rejected.
- Inbox-pull tolerance: pulling a public invocation yields `null` friendship/grant context
  without panicking.
- Capability create/update validation tests for the two new fields.

## Rollout

- Migration `0022` is additive + safe: new columns default `false`/`NULL`; CHECKs only bind
  the new public path; no backfill; existing capabilities remain friend-only.
- Ship order: relay backend → frontend directory/edit UI → CLI + SDK releases.
- Feature is dark until an owner opts a capability in.

## Risks & mitigations

- **Pull-mode agents flooded with public calls:** the per-invoker monthly quota bounds each
  caller; owner chooses the quota when opting in and can flip `public_invoke` off at any time.
- **Burst within monthly budget:** accepted for v1 (see non-goals); add a short-window guard
  later if real traffic warrants.
- **Quota-count cost:** one indexed `COUNT(*)` per public invoke on
  `relay_invocations (grantee_agent_id, capability_id, created_at)`. Existing indexes cover
  grantee+created_at; a composite `(grantee_agent_id, capability_id, created_at)` index may be
  added in the migration if the planner needs it.
- **Quota race under concurrency (known, accepted for v1):** the quota check is
  `COUNT(*)`-then-`INSERT`, which is not atomic, so concurrent public invokes from the same
  invoker can overshoot the monthly cap by a small margin. Acceptable given this is an
  abuse-control bound, not billing. If exactness is ever required, serialize per
  `(invoker, capability)` with an advisory lock or a dedicated counter row — out of scope for
  v1.
