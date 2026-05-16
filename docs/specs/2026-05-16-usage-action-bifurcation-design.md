# Usage page: bifurcate by capability + by platform action

**Status:** design (drafted 2026-05-16, awaiting review)
**Issue:** [#70](https://github.com/Delta-S-Labs/chakra_mcp/issues/70)
**Brainstormed via:** `/brainstorming` + `/systematic-debugging`

## Problem

`/app/usage` (shipped in [#64](https://github.com/Delta-S-Labs/chakra_mcp/pull/64)) shows a total + four dimension rollups: **By API key**, **By agent**, **By pair**, **By org**. A user with only three invocations filed [#70](https://github.com/Delta-S-Labs/chakra_mcp/issues/70) reporting that the page doesn't bifurcate further:

> Usage showed 3 API requests, doesn't show bifurcation based on what kind of request they are. Doesn't show api / pairing / agent based details as well.

Clarified live: the user wants the page to **also** decompose by

1. **Which capability** was called inside those invocations (`message_owner`, `schedule_meeting`, …), and
2. **What kind of platform action** the user performed across the whole network — including non-invocation actions like friendship proposals, grants, agent registrations, capability publishes.

Today the page silently treats "request" as synonymous with "relay invocation." Anything not in `relay_invocations` (friendship lifecycle, grant issuance, capability publish, agent registration) is invisible.

## Goals

- **`By capability`** section on `/app/usage`: per-capability count within the date window.
- **`By platform action`** section: nine rows covering the user's actions across the network — `Inbox invocations`, `Friendships proposed/accepted/rejected/cancelled`, `Grants issued/revoked`, `Agents registered`, `Capabilities published`.
- **Personal ↔ Org scope toggle** on the `By platform action` section header, defaulting to `Personal` (only actions the signed-in user performed) with `Org` showing every member of the user's accounts. URL-state via `?scope=personal|org`.
- All counts respect the existing 7d / 30d / 90d range picker.

## Non-goals

- **HTTP request log.** Reads (listing friendships, opening pages, the dashboard refreshing) are out of scope. Only deliberate writes count.
- **OAuth-grant and discovery-search counts.** No existing table tracks these; deferred to a future `platform_events` table if demand surfaces.
- **Per-capability sparklines.** Table-only for v1.
- **Cross-org rollups.** Page stays scoped to the signed-in user's account memberships.
- **Mutating the existing four sections.** Their queries and copy are unchanged.

## Approach

Two PRs in sequence.

### PR A — schema: attribute platform actions to a user

The existing tables for friendships / grants / agents / capabilities carry the *agent* that performed the action but not the *user*. Without user attribution, the `Personal` scope can't be computed — the toggle would collapse into a "this section only" gimmick.

Add nullable `*_user_id` columns to four tables:

```sql
-- Migration 0019_action_attribution.sql
ALTER TABLE friendships          ADD COLUMN IF NOT EXISTS proposer_user_id  UUID REFERENCES users(id);
ALTER TABLE grants               ADD COLUMN IF NOT EXISTS granter_user_id   UUID REFERENCES users(id);
ALTER TABLE grants               ADD COLUMN IF NOT EXISTS revoked_by_user_id UUID REFERENCES users(id);
ALTER TABLE agents               ADD COLUMN IF NOT EXISTS created_by_user_id UUID REFERENCES users(id);
ALTER TABLE agent_capabilities   ADD COLUMN IF NOT EXISTS created_by_user_id UUID REFERENCES users(id);
```

Pre-migration rows stay `NULL` — same shape as the `api_key_id` and `minted_jti` migrations (0016, 0018) before them. The `Personal` view counts only rows where the column equals the caller's user id; `Org` view ignores the column and falls back to membership-scoping. So old rows degrade gracefully:

- `Org` view: full historical accuracy
- `Personal` view: counts only the post-migration window of activity the user did themselves

Backend handlers writing each of those tables get one extra bind. Sites to touch (verified during research):

- `backend/app/src/handlers/friendships.rs::propose` → set `proposer_user_id = caller`
- `backend/app/src/handlers/friendships.rs::accept|reject|cancel` → write to a new `decided_by_user_id` column? **Decision:** no — the four states are already in the row's `status` field plus their `*_at` timestamps. The proposer_user_id column alone is enough to attribute every friendship-lifecycle row to a personal actor for the four `By platform action` rows: a row counts as "I proposed it" when `proposer_user_id = me`, and counts as "I accepted/rejected/cancelled it" when the matching agent on my side issued the status change. *(This needs a second look during PR A — see Open question 1 below.)*
- `backend/app/src/handlers/grants.rs::create` → `granter_user_id = caller`
- `backend/app/src/handlers/grants.rs::revoke` → `revoked_by_user_id = caller`
- `backend/app/src/handlers/agents.rs::create` → `created_by_user_id = caller`
- `backend/app/src/handlers/capabilities.rs::create` → `created_by_user_id = caller`

`sqlx prepare` updated for each touched query. No relay-side changes (the relay only writes `relay_invocations`, which already has `invoked_by_user_id`).

### PR B — UI: new sections + summary endpoint extension

#### Backend

Extend `GET /v1/usage/summary` (in `backend/app/src/handlers/usage.rs`) to return two new fields:

```rust
struct UsageSummary {
    // ... existing fields ...
    pub by_capability: Vec<CapabilityRollup>,
    pub by_action: ActionBreakdown,
}

struct CapabilityRollup {
    pub name: String,
    pub requests: i64,
}

struct ActionBreakdown {
    pub scope: ActionScope,              // "personal" | "org"
    pub inbox_invocations: i64,
    pub friendships_proposed: i64,
    pub friendships_accepted: i64,
    pub friendships_rejected: i64,
    pub friendships_cancelled: i64,
    pub grants_issued: i64,
    pub grants_revoked: i64,
    pub agents_registered: i64,
    pub capabilities_published: i64,
}
```

`scope` is read from a new optional query param `?scope=personal|org` (default `personal`). The handler dispatches to one of two helpers depending on scope; both helpers issue six `count(*)` queries against the relevant tables.

`by_capability` is a single `GROUP BY capability_name ORDER BY count DESC LIMIT 5` query against `relay_invocations`, with the existing date filter. Top-5 truncation matches the per-key chart's pattern.

#### Frontend

Extend `frontend/src/app/(app)/app/usage/UsageView.tsx` and `frontend/src/lib/api.ts`:

```tsx
// New section, after "By API key":
<Section
  title="By capability"
  empty="No invocations in this window."
  rows={summary.by_capability}
  render={(r) => <td><code>{r.name}</code></td>}
  count={(r) => r.requests}
/>

// New section, at the bottom:
<ActionSection
  scope={summary.by_action.scope}   // controlled by URL state
  counts={summary.by_action}
/>
```

`ActionSection` is a new component that renders the nine-row table plus a `<select>` toggle at the header:

```
By platform action          [ Personal ▼ ]
─────────────────────────────────────────
Inbox invocations                      3
Friendships proposed                   2
Friendships accepted                   1
Friendships rejected                   0
Friendships cancelled                  0
Grants issued                          1
Grants revoked                         0
Agents registered                      1
Capabilities published                 4
```

Toggling the `<select>` updates the URL via `router.push("?scope=…")`. The page re-renders server-side, re-fetching `/v1/usage/summary?scope=…`.

Zero-count rows render at row-level 0, not hidden — the schedule of categories itself is information.

#### Wire shape

```jsonc
GET /v1/usage/summary?range=30d&scope=personal
{
  "from": "2026-04-16T00:00:00Z",
  "to":   "2026-05-16T00:00:00Z",
  "total":     { "requests": 3, "succeeded": 3, "failed": 0 },
  "by_org":    [ ... ],
  "by_agent":  [ ... ],
  "by_api_key":[ ... ],
  "by_pair":   [ ... ],
  "by_capability": [
    { "name": "message_owner",    "requests": 2 },
    { "name": "schedule_meeting", "requests": 1 }
  ],
  "by_action": {
    "scope": "personal",
    "inbox_invocations":       3,
    "friendships_proposed":    2,
    "friendships_accepted":    1,
    "friendships_rejected":    0,
    "friendships_cancelled":   0,
    "grants_issued":           1,
    "grants_revoked":          0,
    "agents_registered":       1,
    "capabilities_published":  4
  },
  "daily": [ ... ]
}
```

## Testing

Backend (`backend/app/src/handlers/usage.rs`):

- New `sqlx::test` for `by_capability` aggregation with three invocations across two capabilities.
- New `sqlx::test` for `by_action` in `personal` scope (caller's actions only) on a seeded mixed-actor account.
- New `sqlx::test` for `by_action` in `org` scope (every member's actions visible) on the same fixture.
- Regression: existing tests for the four current dimensions still green.

Frontend: TypeScript compile + ESLint + Next build.

## Open questions

1. **Friendship acceptance attribution.** The `proposer_user_id` column attributes the *proposal* row to a user. But "Friendships accepted" needs a different attribution: who accepted? The schema doesn't track this. Three options:
   - Add `decided_by_user_id` on `friendships` (one more column on migration 0019). Cleanest.
   - Infer from `target_agent_id`'s owning user via `account_memberships`. Loses precision in multi-member accounts (anyone in the account could have clicked accept).
   - Show `accepted/rejected/cancelled` rows in `Org` scope only; hide them in `Personal`. Punts the question.

   **Proposed:** add `decided_by_user_id` to keep the attribution clean and symmetric. Tiny addition.

2. **Backfill?** Pre-migration rows have `NULL` for the new columns, so `Personal` view shows zero for anything older than the migration. Same precedent as `relay_invocations.api_key_id` (#54) and `minted_jti` (#64): no backfill. Worth a one-line copy callout under the section header — *"Personal-attribution data populated from {migration date} onward; older activity shows only in Org view."*

3. **What about deleted rows?** Tombstoned agents and revoked grants still contributed activity in their lifetime — they should still count. Default: include them, query on `created_at` not on current liveness.

## Out of scope (logged as backlog)

- **Sparklines per category** — defer until v2 if anyone asks.
- **Drill-down from a row to a filtered audit-log view** — useful but is a separate "Audit" feature.
- **OAuth-grant / discovery-search tracking** — add a `platform_events` table later only if needed.

## Rollout

1. Merge PR A (schema). Wait for CD to apply migration 0019.
2. Merge PR B (UI). Netlify deploys the new sections.
3. Verify on prod: `/v1/usage/summary?scope=personal` returns the new fields; `/app/usage` renders both new sections; toggle changes the by_action numbers.
4. Close issue #70 with a comment linking to the page section.

## Why this matters

Three invocations in a 30-day window is a small number, but the reporter's frustration is real: the page promised to surface activity and didn't surface what the user did *with* the platform — only what the platform did *to* their agents. After this lands, the page tells a complete story: *here's what kind of work happened, here's what you did to make it happen, here's how it splits between you and your teammates.*
