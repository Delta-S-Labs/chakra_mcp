# Usage page: bifurcate by capability + by platform action

**Status:** design (drafted 2026-05-16, revised after spec-review iteration 1)
**Issue:** [#70](https://github.com/Delta-S-Labs/chakra_mcp/issues/70)
**Brainstormed via:** `/brainstorming` + `/systematic-debugging`

## Problem

`/app/usage` (shipped in [#64](https://github.com/Delta-S-Labs/chakra_mcp/pull/64)) shows a total + four dimension rollups: **By API key**, **By agent**, **By pair**, **By org**. A user with only three invocations filed [#70](https://github.com/Delta-S-Labs/chakra_mcp/issues/70) reporting that the page doesn't bifurcate further:

> Usage showed 3 API requests, doesn't show bifurcation based on what kind of request they are. Doesn't show api / pairing / agent based details as well.

Clarified live: the user wants the page to **also** decompose by

1. **Which capability** was called inside those invocations (`message_owner`, `schedule_meeting`, …), and
2. **What kind of platform action** the user (or their org-mates) performed across the whole network — including non-invocation actions like friendship proposals, grants, agent registrations, capability publishes.

Today the page treats "request" as synonymous with "relay invocation." Anything not in `relay_invocations` (friendship lifecycle, grant issuance, capability publish, agent registration) is invisible.

## Goals

- **`By capability`** section on `/app/usage`: per-capability count within the date window. Top 20 capabilities, mirroring the convention used by other rollup sections.
- **`By platform action`** section: nine rows covering activity across the network — `Inbox invocations`, `Friendships proposed/accepted/rejected/cancelled`, `Grants issued/revoked`, `Agents registered`, `Capabilities published`.
- **Org ↔ Personal scope toggle** on the `By platform action` section header. URL state via `?scope=personal|org`. Default `org` (see [Decision 3](#decision-3-default-scope) for why this flipped from `personal`).
- All counts respect the existing 7d / 30d / 90d range picker (and survive its URL-state effect — verified safe in current code).

## Non-goals

- **HTTP request log.** Reads (listing friendships, opening pages, the dashboard refreshing) are out of scope. Only deliberate writes count.
- **OAuth-grant and discovery-search counts.** No existing table tracks these uniformly; deferred to a future `platform_events` table if demand surfaces.
- **Per-capability sparklines.** Table-only for v1.
- **Cross-org rollups.** Page stays scoped to the signed-in user's account memberships.
- **Mutating the existing four sections.** Their queries and copy are unchanged.

## Approach

Three PRs in sequence — splitting the schema into its own micro-PR keeps the rollout safe against partial deploys (the original two-PR plan let PR B's `sqlx::query!` macros compile-time-check against an unmigrated DB).

### PR 1 — migration only

`backend/migrations/0019_action_attribution.sql`:

```sql
-- Attribute platform actions to the user who performed them, so the
-- /app/usage "Personal" scope can distinguish my activity from my
-- teammates'. Pre-existing rows stay NULL — same shape as the
-- api_key_id (#54, migration 0016) and minted_jti (#64, migration
-- 0018) attribution columns. Personal-scope queries treat NULL as
-- "not attributable" and exclude. Org-scope queries ignore the
-- column entirely and stay membership-scoped.
--
-- All five columns are nullable on purpose. Many INSERT call sites
-- in the relay are *shadow row creation* — when the relay encounters
-- a friendship/grant/capability that came in via an A2A peer push,
-- the forwarder, or the inbox bridge, it creates a local
-- representation with no authenticated human in the request. Those
-- sites continue to write NULL (documented per-site in PR 2).
ALTER TABLE friendships        ADD COLUMN IF NOT EXISTS proposer_user_id    UUID REFERENCES users(id);
ALTER TABLE friendships        ADD COLUMN IF NOT EXISTS decided_by_user_id  UUID REFERENCES users(id);
ALTER TABLE grants             ADD COLUMN IF NOT EXISTS granter_user_id     UUID REFERENCES users(id);
ALTER TABLE grants             ADD COLUMN IF NOT EXISTS revoked_by_user_id  UUID REFERENCES users(id);
ALTER TABLE agents             ADD COLUMN IF NOT EXISTS created_by_user_id  UUID REFERENCES users(id);
ALTER TABLE agent_capabilities ADD COLUMN IF NOT EXISTS created_by_user_id  UUID REFERENCES users(id);
```

The `decided_by_user_id` on `friendships` resolves [Decision 1](#decision-1-friendship-acceptance-attribution): it captures who clicked accept/reject/cancel and is written by the relay's `accept|reject|cancel` handlers in PR 2.

This migration alone changes no queries; merging it does not require any sqlx cache update. CD applies it before PR 2's binary deploy.

### PR 2 — handler attribution writes

Two crates touch the tables. The split matters: agent CRUD has a second writer outside the relay (the OAuth device-flow sign-in path auto-creates an agent), so the column write needs to land in both.

**Map of every production INSERT/UPDATE site** (verified by `grep -nE '#\[cfg\(test\)\]' …` against the relay and app crates — earlier revisions of this spec over-counted by including test fixtures, which sit inside `#[cfg(test)] mod tests { … }` blocks and don't affect production attribution).

**INSERT sites — all user-attributable, all carry `AuthUser`:**

| Table | Site | Bind |
|---|---|---|
| `agents` | `backend/relay/src/handlers/agents.rs:357` (in `pub async fn create`) | `created_by_user_id = user.user_id` |
| `agents` | `backend/app/src/handlers/oauth.rs:958` (device-flow approve auto-creates the paired agent) | `created_by_user_id = subject_user_id` (the user approving the pair) |
| `friendships` | `backend/relay/src/handlers/friendships.rs:288` (in `pub async fn propose`) | `proposer_user_id = user.user_id` |
| `friendships` | `backend/relay/src/handlers/friendships.rs:491` (in `pub async fn counter`) | `proposer_user_id = user.user_id` |
| `friendships` | `backend/relay/src/handlers/mcp.rs:806` (in `pub async fn handle`, MCP tool can propose a friendship as a side-effect; MCP requests are bearer-authed and carry `AuthUser`) | `proposer_user_id = user.user_id` |
| `grants` | `backend/relay/src/handlers/grants.rs:323` (in `pub async fn create`) | `granter_user_id = user.user_id` |
| `agent_capabilities` | `backend/relay/src/handlers/capabilities.rs:158` (in `pub async fn create`) | `created_by_user_id = user.user_id` |

**UPDATE sites — status transitions on existing rows:**

| Table | Site | Bind |
|---|---|---|
| `friendships` | `backend/relay/src/handlers/friendships.rs:313` (`pub async fn cancel`) | `decided_by_user_id = user.user_id` alongside `status = 'cancelled'` |
| `friendships` | `backend/relay/src/handlers/friendships.rs:353` (`pub async fn accept`) | `decided_by_user_id = user.user_id` alongside `status = 'accepted'` |
| `friendships` | `backend/relay/src/handlers/friendships.rs:397` (`pub async fn reject`) | `decided_by_user_id = user.user_id` alongside `status = 'rejected'` |
| `grants` | `backend/relay/src/handlers/grants.rs:386` (`pub async fn revoke`, UPDATE setting `revoked_at`) | `revoked_by_user_id = user.user_id` |

**Note on shadow paths.** The relay codebase does not currently have production-side automatic creation of `friendships` / `grants` / `agent_capabilities` rows from non-user contexts. All the apparently-shadow INSERTs I initially flagged (`forwarder.rs:496/509`, `a2a.rs:589/601/612`, `inbox_bridge.rs:310/321`, `invoke.rs:971/986/998`, `published_cards.rs:420`, `discovery.rs:488`) live inside `#[cfg(test)] mod tests` blocks and are test fixtures only. So every production write touches a user — every bind site above is straightforward.

**Test fixtures.** Test code that constructs these rows continues to work without changes: the new `*_user_id` columns are nullable, so test INSERTs that don't bind them get NULL — same as production rows older than the migration. The new sqlx::test cases in PR 3 for the by_action scope queries will need to bind the user columns explicitly (they're exercising the new logic).

`sqlx prepare` cache regenerated for each touched query; pre-commit hook (`sqlx-prepare-check`) gates this. PR 2's binary deploys only AFTER PR 1's migration applies — existing CD ordering runs `task migrate` before the image swap.

### PR 3 — UI: new sections + summary endpoint extension

#### Backend

Extend `backend/app/src/handlers/usage.rs`. The existing struct is **`SummaryResponse`** (not `UsageSummary` as my first draft said), and the existing query-param struct is **`RangeQuery`**. Both get extended; the new `scope` field defaults to `Org`:

```rust
#[derive(Deserialize)]
pub struct RangeQuery {
    pub from: Option<DateTime<Utc>>,
    pub to:   Option<DateTime<Utc>>,
    #[serde(default)]
    pub scope: ActionScope,   // NEW
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ActionScope {
    #[default]
    Org,         // serialised as "org"
    Personal,    // serialised as "personal"
}

#[derive(Serialize)]
pub struct SummaryResponse {
    pub from: DateTime<Utc>,
    pub to:   DateTime<Utc>,
    pub total:     TotalRollup,
    pub by_org:    Vec<OrgRollup>,
    pub by_agent:  Vec<AgentRollup>,
    pub by_api_key: Vec<ApiKeyRollup>,
    pub by_pair:   Vec<PairRollup>,
    pub daily:     Vec<DailyBucket>,
    pub by_capability: Vec<CapabilityRollup>,   // NEW
    pub by_action:     ActionBreakdown,         // NEW
}

#[derive(Serialize)]
pub struct CapabilityRollup {
    pub name: String,
    pub requests: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ActionBreakdown {
    pub scope: ActionScope,    // echoes the query param so the UI can confirm what it got
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

The `by_action` query branches on `scope`:

- **`Org`**: every count joins through `account_memberships` to scope to the caller's accounts. Ignores the new `*_user_id` columns entirely. Works on all rows including pre-migration history.
- **`Personal`**: every count adds `AND <table>.<actor>_user_id = $caller` on top of the org scope. Post-migration rows attributed to the caller only.

The 9 counts are six issuing-side `count(*)` queries (one per friendship status, one for grants_issued, agents_registered, capabilities_published) and three transition-side counts (grants_revoked via `revoked_at IS NOT NULL AND revoked_by_user_id = $caller`, and the friendship accept/reject/cancel counts via `decided_by_user_id`). All bounded by the existing `from`/`to` window.

`by_capability` is a single `SELECT capability_name, COUNT(*) FROM relay_invocations WHERE … GROUP BY capability_name ORDER BY count DESC LIMIT 20` matching the top-20 convention used by `by_agent` and `by_api_key` in the existing handler.

#### Frontend

Two new sections on `frontend/src/app/(app)/app/usage/UsageView.tsx`. Reuse the existing `Section<T>` component (real signature: `{ title, empty, rows, renderRow, headers }`):

```tsx
<Section
  title="By capability"
  empty="No invocations in this window."
  headers={["Capability", "Requests"]}
  rows={summary.by_capability}
  renderRow={(r) => (
    <>
      <td><code>{r.name}</code></td>
      <td className={styles.numericCol}>{r.requests.toLocaleString()}</td>
    </>
  )}
/>
```

The `By platform action` section is new visual chrome (a small `<select>` toggle in the header) so it doesn't reuse `Section`; it gets its own `ActionSection` component rendering a fixed nine-row table. The toggle uses `router.replace(...)` to flip `?scope=`. The existing range-picker `useEffect` at `UsageView.tsx:44-49` builds the URL from `window.location.href` and only `set`s/`delete`s `range`, so it preserves an existing `?scope=` — no collision (verified).

`ActionSection` is also responsible for the Decision-4 footnote: when `scope === "personal"`, render a one-liner under the table — *"Personal-attribution data populated from {migration apply date} onward. Older activity shows only in Org view."* Render nothing under `Org`. Tooltip on the toggle's header carries the Security note (*"Org view counts every member's activity, not just yours."*).

```
By platform action          [ Org ▼ ]   ← toggle
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

Zero-count rows are deliberately shown — see [Decision 2](#decision-2-zero-rows).

A `getUsageSummary` helper in `frontend/src/lib/api.ts` learns the optional `scope` param and the two new fields on the response type.

#### Wire shape

```jsonc
GET /v1/usage/summary?range=30d&scope=org
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
    "scope": "org",
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

## Decisions

(Three open questions from the previous draft, resolved.)

### Decision 1 — friendship-acceptance attribution

The original spec proposed `proposer_user_id` only. Reviewer correctly pointed out this misses the accept/reject/cancel attribution, plus several non-user pathways (A2A peer push, MCP, background forwarder, inbox bridge).

**Decision:** add **two** columns on `friendships`: `proposer_user_id` (set on INSERT) and `decided_by_user_id` (set on the status-change UPDATE in accept/reject/cancel handlers). Both stay NULL on shadow-creation paths per the policy table above. For attribution purposes:

- "Friendships proposed by me" counts rows where `proposer_user_id = me`.
- "Friendships accepted by me" counts rows where `decided_by_user_id = me AND status = 'accepted'`. Same for rejected/cancelled.

**Multi-actor pathways spelled out:**
- Accept via web UI (JWT): `decided_by_user_id = JWT subject`.
- Accept via API key (`ck_…`): the key's `user_id` column maps cleanly; set `decided_by_user_id = key.user_id`.
- Accept via background bridge/forwarder/A2A push: no user; leave NULL. Counts toward Org scope only.
- Accept via peer-pushed A2A SendMessage that auto-accepts a pending request: NULL; same reasoning.

### Decision 2 — zero rows

The 9 rows in `By platform action` render even when count is 0. Rationale: the *menu of categories* is the information — telling the user "here are the things the platform tracks" matters as much as the numbers. The page already does this for the daily chart's zero-bar days.

(The reviewer flagged that a brand-new user with all-zero Personal sees nine zeros. Default scope is now `Org`, which mitigates — see Decision 3.)

### Decision 3 — default scope

**Flipped from `Personal` to `Org`.** Reasons:

1. A new user landing on the page with no Personal-attributed activity (pre-migration history doesn't carry attribution) sees all-zero Personal rows. Confusing.
2. `Org` view is consistent with the existing four sections' membership scoping. Less cognitive load.
3. Most users are in a single personal account where Org and Personal are identical. The toggle only meaningfully diverges in multi-member orgs, where the *teammate's* activity is the surprising-then-illuminating thing.

`Personal` remains a one-click toggle and the URL param ships in deep-links.

### Decision 4 — backfill

No backfill of pre-migration rows. Same precedent as `api_key_id` (#54) and `minted_jti` (#64). The Org scope handles the historical case correctly; Personal scope is honest-low for older accounts. Add a one-line footnote below the section in `Personal` mode only: *"Personal-attribution data populated from {migration apply date} onward."*

### Decision 5 — top-N truncation on By capability

LIMIT 20, matching the existing `by_agent` and `by_api_key` conventions in the same handler. The wire example shows 2 rows because that's the realistic count for a fresh account; no overflow affordance needed at 2 rows. If the user hits 21+ capabilities we can add a "+ N more" link later.

## Security note

`Org` scope intentionally surfaces every member's activity to every other member of the same account. This is information disclosure within an org and is intentional — billing-level accounts already share visibility today (agents, grants, audit log). A contractor added to an account will see how many friendships their teammates proposed in the window. Worth flagging in the section header tooltip: *"Org view counts every member's activity, not just yours."*

## Testing

Backend (`backend/app/src/handlers/usage.rs`):

- New `sqlx::test` for `by_capability` aggregation with three invocations across two capabilities.
- New `sqlx::test` for `by_action` in `Personal` scope — seeded mixed-actor account, verify caller's actions only.
- New `sqlx::test` for `by_action` in `Org` scope — same fixture, verify all members' actions visible.
- New `sqlx::test` for the friendship `decided_by_user_id` UPDATE path — accept changes the value, reject changes it differently, cancel works.
- Regression: existing tests for the four current dimensions still green; existing `by_pair` / `by_org` / `by_api_key` / `by_agent` queries unchanged.

Frontend: TypeScript compile + ESLint + Next build. Manual deploy-preview smoke: visit `/app/usage?scope=org&range=30d`, toggle to `personal`, confirm `?scope=personal&range=30d` and the numbers change.

## Rollout

1. **PR 1** lands. Migration 0019 applies in CD via `task migrate` before the image swap. Verify with `psql -c '\\d friendships'` that the new columns exist.
2. **PR 2** lands. New `sqlx prepare` cache (covering the modified INSERT/UPDATE bindings) is committed in the same PR. CI catches drift via the existing `Verify .sqlx cache is up to date` job.
3. **PR 3** lands. Netlify deploys the new sections; backend deploys the extended endpoint.
4. Smoke: `curl https://app.chakramcp.com/v1/usage/summary?scope=org` (with a valid token) returns 200 + the new fields populated; `/app/usage` renders both new sections + a working toggle.
5. Close [#70](https://github.com/Delta-S-Labs/chakra_mcp/issues/70) with a comment linking to the page.

## Out of scope (logged as backlog)

- **Sparklines per category** — defer until v2 if anyone asks.
- **Drill-down from a row to a filtered audit-log view** — useful but is a separate "Audit" feature.
- **OAuth-grant / discovery-search tracking** — add a `platform_events` table later only if needed.
- **Per-action sub-attribution for shadow rows** — the policy says "NULL on shadow paths." If we later want to attribute peer-push shadow rows to the *originating* peer agent, that's a separate spec (and a separate column on the row).

## Why this matters

Three invocations in a 30-day window is a small number, but the reporter's frustration is real: the page promised to surface activity and didn't surface what the user did *with* the platform — only what the platform did *to* their agents. After this lands, the page tells a complete story: *here's what kind of work happened, here's what platform actions made it possible, and (in multi-member orgs) here's how it splits between you and your teammates.*
