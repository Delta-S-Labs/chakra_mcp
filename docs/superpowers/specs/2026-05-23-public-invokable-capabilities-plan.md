# Public-Invokable Capabilities — Implementation Plan

Derived from `2026-05-23-public-invokable-capabilities-design.md` (approved).
Phases are in dependency order. Suggested PR boundaries noted; each PR is independently
green-able and reviewable.

## PR 1 — Backend: schema + invoke path + quota + inbox HITL fix + tests

The cohesive backend slice. Everything here ships together because the invoke path depends on
the schema and the tests exercise the whole path.

### Phase 1.1 — Migration `0022_public_invokable_capabilities.sql`
- Add `public_invoke BOOLEAN NOT NULL DEFAULT false` and
  `public_monthly_quota_per_agent INTEGER` to `agent_capabilities`.
- Add CHECKs `cap_public_requires_network` and `cap_public_requires_quota` (per spec).
- Optional composite index `(grantee_agent_id, capability_id, created_at)` on
  `relay_invocations` if the quota `COUNT(*)` plan needs it (decide after EXPLAIN).
- Apply locally; `cargo sqlx prepare --workspace -- --tests`.
- **Verify:** psql `\d agent_capabilities` shows columns + CHECKs; insert with
  `public_invoke=true, visibility!='network'` is rejected; with NULL quota rejected.

### Phase 1.2 — Capability owner controls (relay `handlers/capabilities.rs`)
- Extend `CreateCapabilityRequest` + `UpdateCapabilityRequest` with optional
  `public_invoke: Option<bool>` and `public_monthly_quota_per_agent: Option<i32>`.
- Validation → `400`: if effective `public_invoke = true`, require `visibility = network`
  and quota `>= 1`.
- PATCH keeps COALESCE semantics.
- Add the two fields to the `CapabilityDto` returned by create/update/list.
- **Verify:** unit tests for each validation branch.

### Phase 1.3 — Invoke path public mode (relay `handlers/invoke.rs`)
- `InvokeRequest`: make `grant_id: Option<Uuid>`, add `capability_id: Option<Uuid>`. Reject
  both-set / neither-set with `400`.
- Public branch: membership check on `grantee_agent_id`'s account (reuse existing); resolve
  `capability_id` → must be `network` + `public_invoke=true` else `404`; self-invoke guard
  (`grantee == granter`) → `400`; monthly quota `COUNT(*)` → `429` with
  `{error:"monthly_quota_exhausted", quota, resets_at}`; enqueue with `grant_id = NULL`.
- Trusted branch unchanged.
- **Verify:** the test list below.

### Phase 1.4 — Inbox HITL semantics fix (relay `handlers/invoke.rs` inbox-pull)
- When the pulled invocation has `grant_id IS NULL`, source `semantics` + `capability_name`
  from `i.capability_id` (the invocation's own column / a join on the capability directly),
  not through the grant join.
- Confirm `friendship_context` / `grant_context` emit `null` for NULL grant (already the
  case — add the test).

### Phase 1.5 — Backend tests (`sqlx::test`)
- Public invoke by non-friend on public cap → enqueues `grant_id NULL`, correct grantee/granter.
- Non-public / non-network cap via public mode → `404`.
- Quota: at cap → `429`; prior-month rows excluded; consumed on enqueue even when downstream
  `failed`.
- Validation: both ids → `400`; neither → `400`; self-invoke → `400`; invoke-as-not-mine → `403`.
- DB CHECKs rejected as expected.
- Inbox pull of a public invoke: null trust contexts + **correct `semantics`** for a
  `human_in_loop` public capability.
- `cargo test -p chakramcp-relay`; `cargo fmt`; sqlx-prepare check.

### Phase 1.6 — Discovery exposure (relay `handlers/discovery.rs`)
- Add `public_invoke` + `public_monthly_quota_per_agent` to the capability rows in
  `GET /v1/discovery/agents` and the authed agent-detail/capability listing.

## PR 2 — Frontend
- `/agents` directory + agent detail + `/app/agents/network`: render a `public · N/mo` marker
  on publicly-invokable capabilities.
- Capability create/edit UI under `/app/agents`: "Publicly invokable" toggle + monthly-quota
  input, shown only when capability `visibility = network` (matches the CHECK); client + server
  validation.
- `tsc` + `eslint`; preview-verify the toggle gating + marker.

## PR 3 — CLI (`chakramcp`)
- `capabilities add` / `capabilities update`: `--public-invoke`, `--monthly-quota <N>`.
- `invoke`: `--capability <id>` as an alternative to `--grant <id>`.
- `discover` / capability listing: `public · N/mo` marker.
- Verify against dev backend; `cargo clippy`.

## PR 4 — SDKs (Python, TypeScript, Rust, Go)
- `Capability` type: `public_invoke`, `public_monthly_quota_per_agent`.
- Create/update-capability requests: the two optional fields.
- `invoke` public-mode form taking `capability_id`.
- Typed `QuotaExhaustedError` (quota + resets_at) on `429`.
- Build + test each SDK.

## Releases (post-merge, not code)
- `cli-v*`, `sdk-py-v*`, `sdk-ts-v*` bump + tag → existing release workflows.
- Rust/Go SDKs via their own flows.

## Sequencing notes
- PR 1 is the critical path; PRs 2–4 depend only on PR 1's API contract and can proceed in
  parallel once it merges.
- Feature is dark until an owner opts a capability in, so partial rollout (backend merged,
  frontend/CLI/SDK trailing) is safe.
- After this lands, start the **sub-project 2** spec (ratings/reviews) — it reads the
  `relay_invocations` rows this produces.
