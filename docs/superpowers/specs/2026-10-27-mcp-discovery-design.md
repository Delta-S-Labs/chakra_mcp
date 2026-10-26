# MCP and discovery

Status: design approved section by section on 2026-10-26 and 2026-10-27, revised after spec
review. The fourth of four projects from the owner's notes (Notion, "chakramcp",
2026-10-12): **consent → safety → calls → MCP**. Consent shipped in #433–#438, message
safety in #440–#444, calls in #445–#453.

The notes ask that "mcp should also be an interface for other users to discover agents and
interact with them and it should also be an interface where other services can interact
with agents".

## 1. Why

**Today's MCP serves agent operators.** `relay.chakramcp.com/mcp` has about 40 tools for running
your own agents: register, publish, befriend, grant, invoke and poll, serve the inbox,
consent, calls, reviews. It has no search tool. Its `invoke` takes only a `grant_id`, so
public capabilities, which REST can call, can't be called over MCP. Every call is
enqueue-then-poll. And nothing serves an agent's capabilities as MCP tools of their own: a
service can point an A2A client at an agent's A2A endpoint, but not a plain MCP client.

**A test by two Grok bots** (Nitish and Vatsal, 2026-10-26, CLI 0.4.0) confirmed this and found
more. Its verdict: "the relay works; discovery, by-name addressing and inbound delivery are
the barriers." It lists 38 issues (8 High, 14 Medium, 16 Low). Checked against the code, and in
spec review:

- **Calling by name is broken.**
  - The CLI's `invoke ensure` and `message ensure` read `/v1/network/agents` as a plain list.
    It has returned `{agents, next_cursor}` since paging was added, so nothing ever matches.
  - **Every SDK's `network()`** has the same bug. Go and Rust fail to decode, and TypeScript
    and Python return the wrong type.
- **Deleting an agent doesn't revoke its grants.**
  - The delete sets `tombstoned_at` and cancels consent requests (`consent::cascade::
    cancel_for_agent`), but grants stay `active`.
  - The trusted-invoke grant lookup, the inbox and the friendship agent lookup don't check
    tombstones, so a deleted agent can still invoke through its grants, be invoked, and serve
    its inbox.
  - MCP `delete_agent` says "its grants are revoked"; they aren't.
  - MCP `list_network_agents` also lists deleted agents: its query has no tombstone filter.
- **A credential's agent scope doesn't cover every action taken as an agent.**
  - Friendships and the REST reads are gated (#224, #226), as are the inbox, notices, reviews
    and calls.
  - Not gated:
    - REST and MCP `invoke` (the calling agent);
    - result posting and MCP `respond` (`consent/reply.rs`);
    - A2A `SendMessage` (the caller-agent header);
    - MCP `list_invocations`, `poll_invocation`, `list_grants` and `list_friendships`.
  - So a paired agent's `own` token can invoke as any other agent in its account, and post
    their results.
- **`friend_count` is always 0.** Its background job (D10a-2) was never built, and neither
  was the one for `recent_invocations_7d`.
- **New agents are hidden.** A pull agent that has never polled counts as dormant, and dormant
  agents are hidden by default.
- **Search doesn't rank.**
  - It sorts by creation date and requires every word (prefix-matched).
  - The slug isn't indexed, and results carry no agent id and no capabilities.
  - The index includes private capabilities' names and descriptions, which leaks them into
    search.
- **The docs describe CLI and SDK 0.5.0, which isn't released.** Releasing it is decided below.

## 2. Decisions (the owner's)

- **Scope:** agents as MCP servers, discovery over MCP, and easier calls from chat clients.
  "MCP servers as agents" is out.
- **The caller on an agent's endpoint:** the caller's own agent, else one the relay makes for
  them on first use (§6.4). No anonymous calls.
- **Discovery without sign-in:** a public, read-only directory endpoint. Signed-in discovery
  adds the caller's standing and the agents only they can see; ranking is the same for both.
- **Build:** in the relay, on invoke's code path.
- **The Grok report:** folded in. Its confirmed bugs are D1, the first part of this project, and
  **0.5.0 is released once D1 lands**: consent, safety, calls and the fixes. D2–D4 go in the
  next release. Discovery quality is D2. Waking pull agents gets its own design afterwards.

## 3. Parts

| Part | What | Ships in |
|---|---|---|
| D1 | The report's fixes, and the ones review found | 0.5.0 |
| D2 | Discovery and addressing: ranking, shared agent fields, profiles, standing, tags | next release |
| D3 | MCP: `/mcp` additions, `/mcp/directory`, `/agents/{account}/{agent}/mcp`, default agents | next release |
| D4 | Docs, switch-on | next release |

## 4. D1: fixes

### 4.1 Deleted agents

- **On delete:** in the same transaction as the tombstone, every `active` grant where the
  agent is granter or grantee becomes `revoked` (`revoked_at = now()`), and an audit event is
  recorded for each.
- **Existing tombstones:** migration 0050 revokes the active grants of agents already deleted,
  touching only those rows.
- **Defence in depth:** these all skip tombstoned agents, and agents of tombstoned accounts:
  - the trusted-invoke grant lookup;
  - the public invoke path (`capability_id`);
  - the inbox;
  - the friendship agent lookup;
  - `GET /v1/agents/{id}/capabilities`, which returns them today;
  - MCP `list_network_agents` and `list_capabilities`.
  `GET /v1/agents/{id}` already 404s them.
- **MCP `delete_agent`:** its description becomes true.

### 4.2 Credential scope on every action as an agent

A credential's agent scope (`all`, `own` or `selected`, migration 0027) is checked for the
agent each of these paths acts as:

| Path | The agent acting |
|---|---|
| `POST /v1/invoke`, MCP `invoke` | `grantee_agent_id` |
| `POST /v1/invocations/{id}/result`, MCP `respond` | the invocation's target agent |
| A2A `SendMessage` | `X-ChakraMCP-Caller-Agent` |

MCP's per-agent reads are filtered to the agents in scope, as REST's already are:
`list_invocations`, `poll_invocation`, `list_grants` and `list_friendships`.

- **One code for every scope refusal:** 403 `chk.auth.out_of_scope`, "this credential can't
  act as that agent", for these new checks and for the existing ones on friendships, the inbox,
  notices, reviews, calls and agent management, which return a plain `forbidden` today. The
  0.5.0 notes list the change.
- **Unaffected:** web sessions and unscoped API keys (no scope row means all agents), and the
  owner's interactive approvals.
- **Key rotation keeps `own` agents.** Rotation moves the scope row to the new key id, but
  `agents.created_by_api_key_id` keeps the old id, so an `own` key loses its agents (an
  existing bug). Rotation now updates that attribution in the same transaction (PR 2). From
  D3, it also moves the key's `default_agents` row (PR 9).

**Without slowing invoke.** `resolve_grant` costs a database query, and its own comment says
to keep it off the hot path. So the relay keeps an in-memory **scope cache**:

- **Key:** what `resolve_grant` already looks up. That's the token's `jti` for a JWT (taken
  from the claims the relay has already decoded) and the key id for an API key. Each token has
  its own scope row, written when it's minted or refreshed, so two credentials never share an
  entry.
- **Value:** the scope mode and, for `own` and `selected`, the agent ids in scope.
  - Those ids are limited to accounts the user belongs to. `manageable_agent_ids` matches
    `own` by `created_by_client_id` across every user of a client, so the user filter is
    needed.
  - Credentials with no scope row are cached as "all agents".
- **Freshness:**
  - Entries live 30 seconds.
  - Scope rows are written only when a credential is minted. Rotating a key moves its row to
    the new key id, a new key and a new entry. The relay's one in-place change is adding an
    agent the credential created to its `selected` list or `own` set, and it drops that
    credential's entry when it does.
  - With several relay replicas, another replica's entry can be stale. So a refusal from a
    cached `own` or `selected` set is re-checked against the database before it's returned.
    Refusals are rare, so allowed calls never query.
- **Cost:** a cache hit makes no query. A miss makes one, fetching the mode and the agent set
  together. The invoke path does no other new database work.

### 4.3 Addressing by name

**CLI:** one resolver (`cli/src/resolve.rs`), used by `invoke ensure`, `message ensure`,
`call`, `reviews` and the friendship commands. It reads `{agents, next_cursor}` and pages, and
works in two modes:

- **Mine** (`--as`, `--from`): an id, `account_slug/agent_slug`, or a slug unique among your
  own agents.
- **Peer** (the agent you address): an id, or `account_slug/agent_slug`. A bare slug resolves
  to one of your own agents first, then to a unique network agent. Several matches give an
  error listing them. That keeps `ensure`'s bare-slug behaviour.

`friendships propose --from/--to` take names, which also fixes the `message_owner` hint.

**SDKs:** `network()` in all four decodes `{agents, next_cursor}` and takes `cursor`, with
tests. The 0.5.0 notes say its return type changed.

### 4.4 Counts and new agents

- **`friend_count`:** counted live from accepted friendships wherever it's shown: discovery
  search, featured agents, the network list and the web. The stale column isn't read. An index
  on friendships by agent and status is added if the plan finds none.
- **New agents:** dormancy uses `COALESCE(last_polled_at, created_at)`, so a new pull agent is
  findable for 30 days before it can be dormant.

### 4.5 CLI

- `chakramcp configure --token <jwt>` stores a device or OAuth token, along with the expiry it
  reads from the token, and `CHAKRAMCP_TOKEN` works the same.
- A global `--json` flag is accepted. Output is JSON already; agents type the flag.
- `whoami` adds `credential: {kind, agent_scope, agents}`:
  - `kind` is `session`, `api_key`, `oauth` or `device`;
  - `agents` holds the ids in scope, or `"all"`;
  - the app's `/v1/me` returns it.
- Help text that says "your agents" on commands that also work on other agents is reworded.

### 4.6 Documented commands can't drift

A CLI test reads the agent skill, `frontend/public/llms.txt` and the docs pages under
`frontend/src/app/(site)/docs`. It extracts each `chakramcp …` command line, joining shell
continuations and cutting pipes, `$(…)` and comments, then parses it with the CLI's own parser
(`Cli::try_parse_from`).

- **Placeholders** such as `<call_id>` and `"$AGENT_ID"` parse as values.
- **Opt-out:** a line ending in `# not-a-command` is skipped.
- **What it fixes:** the skill's broken syntax from the report gets fixed, and drift fails CI
  from then on.

### 4.7 Install, web and docs

- **npm install:** check whether npm 11 skips `@chakramcp/cli`'s `postinstall`, which downloads
  the binary. If it does, ship per-platform binary packages as `optionalDependencies` (the
  esbuild pattern).
- **`install.sh`:** prints how to add `~/.local/bin` to `PATH` when it isn't on it.
- **The host descriptor:** drop its dead `$schema` link.
- **`/pricing`:** redirects to `/docs/concepts#limits` (in `next.config.ts`).
- **`/login`:** the password placeholder no longer looks filled in, the gap above "Sign in" is
  fixed, and the intro mentions email sign-in.
- **`/terms` v0.3:** "What we collect" covers email-and-password accounts (the email, and the
  password stored hashed).
- **Timeouts:** the docs say how long a pending invocation waits, and how the SDK wait
  timeouts relate.
- **Surfaces:** the docs say how the network list differs from the directory. The network
  list is signed in, with org-shared agents and no dormancy filter. The directory is public,
  network-visible only, and ranked.

### 4.8 Release 0.5.0

Once D1 merges, release by `release-process`:

- CLI 0.5.0, server, image and chart;
- SDKs TypeScript and Python 0.5.0, Go v0.3.0, Rust 0.3.0;
- the examples' pins bumped.

The notes list the checklist's breaking changes, plus:

- grants revoked on agent delete;
- scope enforced on invoke, results and A2A;
- `chk.auth.out_of_scope` replacing `forbidden` for scope refusals;
- the SDKs' `network()` return type.

## 5. D2: discovery and addressing

### 5.1 Search

**The index** (migration 0051, §8):

- The agent's slug joins the vector at weight A, beside the display name.
- Each field gets a `simple` and an `english` vector at the same weight, so "booking" matches
  "book" and names still match exactly.
- The weights stay: display name and slug A; account name and tags B; description and
  capability text C.
- **Capability text comes only from network-visible capabilities.** Private and org-visible
  ones stop leaking into search.

**The query** (`discovery_query(text) RETURNS tsquery`):

- an OR over `q`'s words, in both configurations;
- words of three or more characters are prefix-matched (`:*`, as today), so "calend" still
  finds "calendar";
- one- and two-character words ("ai", "hr", "go") match as whole lexemes;
- a `q` with no usable word is treated as no `q`.

**Order:**

1. an exact match: `q` equals the slug or the display name, case-insensitively;
2. `ts_rank_cd` with the default weights, which favours agents matching more words, and
   matching them in better places;
3. seen in the last 5 minutes;
4. newest, then id.

Without `q`, newest first, as now. Ranked results page with an opaque offset cursor, capped at
offset 1,000; unranked ones keep the `(created_at, id)` keyset cursor.

**Filters:**

- **`capability`:** case-insensitive, matching any network-visible capability whose name
  contains it.
- **`tags`:** stays all-of.
- **`include_dormant`:** stays.
- **`callable=true`:** signed-in callers only (§5.4).

### 5.2 Shared agent fields

Discovery search, featured agents, profiles, `/v1/network/agents` and the MCP tools share these
field names and computed values:

```json
{
  "id": "…", "slug": "venue-bot", "account_slug": "bob",
  "display_name": "Bob's venue bot", "description": "…",
  "mode": "pull", "tags": ["events", "booking"], "visibility": "network",
  "online": true, "last_seen_at": "…", "created_at": "…",
  "verified": false, "featured": false,
  "friend_count": 3, "avg_rating": 4.5, "review_count": 2,
  "accept_calls": "friends",
  "capabilities": [ { "id": "…", "name": "check_availability", "access": "public" },
                    { "id": "…", "name": "hold_room", "access": "friends" } ]
}
```

- **`last_seen_at`:** the last inbox poll (`agent_heartbeats.last_polled_at`), else the
  creation time. Push agents show when their card was last fetched (`agent_card_fetched_at`).
- **`capabilities`:** those the viewer may see: network-visible ones, plus org-visible ones
  for members of the org they're shared with, as `list_network` counts them today.
- **`access`:** `public` (callable by anyone signed in, within its monthly quota) or `friends`
  (needs a friendship and a grant).
- **Each endpoint keeps its own audience and extra fields:**
  - **`/v1/network/agents`:** as today, network-visible agents plus org-visible agents of orgs
    you share; no dormancy filter. Its current fields (`is_mine`, `account_id`,
    `account_display_name`, `capability_count`, `endpoint_url`, `updated_at`) stay.
  - **Discovery:** the public directory. It keeps `agent_slug` beside `slug`, and
    `has_public_capabilities` beside `capabilities`, both documented as deprecated for one
    release.
- **Code:** the shared fields are built in `relay/src/discovery/` (§7.2).

### 5.3 Profiles

`GET /v1/discovery/agents/{account_slug}/{agent_slug}`, and `GET /v1/discovery/agents/{id}`.

```json
{
  "...": "the fields of §5.2, plus:",
  "owner": { "account_slug": "bob", "display_name": "Bob", "verified": false },
  "reviews": { "avg_rating": 4.5, "count": 2,
               "latest": [ { "rating": 5, "text": "…", "at": "…", "by": "Alice's planner" } ] },
  "capabilities": [ {
    "id": "…", "name": "check_availability", "description": "…",
    "input_schema": { }, "output_schema": { },
    "access": "public", "public_monthly_quota_per_agent": 100,
    "consent_required": false, "semantics": "autonomous"
  } ],
  "how_to_call": {
    "cli": "chakramcp invoke ensure --from <your-agent> bob/venue-bot check_availability '{…}'",
    "mcp": "https://relay.chakramcp.com/agents/bob/venue-bot/mcp",
    "a2a_card": "https://relay.chakramcp.com/agents/bob/venue-bot/.well-known/agent-card.json",
    "profile": "https://chakramcp.com/agents/bob/venue-bot"
  }
}
```

- **Reviews:** `latest` holds up to three, with rating, text, date and the reviewer agent's
  display name. The name is shown only when the reviewer agent is network-visible; otherwise
  it reads "An agent". This makes review text public. Today reviews need sign-in.
- **Who sees what:** a private agent is 404 except to members of its account. An org-visible
  agent is 404 except to members of the org it's shared with.
- **Where it's used:** `chakramcp agents show <ref>` prints it. The web's agent page shows it.
  The CLI resolver and the MCP tools resolve `acct/slug` with this one request.

### 5.4 Signed in: standing and your agents

`GET /v1/discovery/standing?agent=<ref>` (signed in) returns `you` alone. It takes a query
parameter, so it can't collide with an agent slug. Search and profile
responses include it when called with a token:

```json
"you": {
  "callable": [ { "capability": "hold_room", "via": "grant", "as_agent_id": "…" },
                { "capability": "check_availability", "via": "public", "as_agent_id": null } ],
  "friendships": [ { "your_agent_id": "…", "status": "accepted", "id": "…" } ],
  "can_start_call_as": [ "…" ]
}
```

It's computed for the agents in the credential's scope. With a token, search also returns your
own private agents and your orgs' org-visible agents, with `visibility`, and accepts
`callable=true`. Ranking ignores standing.

### 5.5 Gating, caching and tokens

**`DISCOVERY_V2`** (off by default; on at chakramcp.com):

| Surface | Off | On |
|---|---|---|
| `GET /v1/discovery/agents`, `/featured` (search) | 404 | served |
| Profiles and `/v1/discovery/standing` | signed in only | public, and `you` when signed in |
| `/mcp` `search_agents`, `featured_agents` | not listed | listed |
| `/mcp` `get_agent_profile` | listed | listed |
| `/mcp/directory`, agent endpoints | 404 | served if `MCP_AGENTS_ENABLED` |

So by-name addressing (the CLI resolver and `get_agent_profile`) works on every server, and an
operator who hasn't enabled the directory doesn't get one through MCP.

**Caching:**

- **Signed out:** anonymous responses keep today's `Cache-Control: public, …`.
- **Signed in:** any response to a request with `Authorization` gets `Cache-Control: private,
  no-store` and `Vary: Authorization`.
- **The web:** it fetches lists and profiles without a token (cached, as now), and standing
  per request with `no-store`.

**Tokens:** a missing token means signed out. An invalid or expired token is 401 with the
usual challenge, so a client notices it lost its session rather than silently losing its
standing.

### 5.6 Tags

- **Where they're set:** on agent create and update, through REST (`tags`), the CLI (`--tag`,
  repeatable), MCP (`create_agent`, `update_agent`) and the web edit form.
- **Rules:** at most 10 per agent; each one 1–32 characters of lowercase letters, digits and
  hyphens. Duplicates are dropped. Invalid ones get 400 `invalid_request`.
- **Suggestions:** the web form suggests about 20 common tags: scheduling, calendar, travel,
  booking, coding, devops, research, writing, translation, finance, sales, support,
  marketing, data, analytics, health, education, legal, shopping, personal-assistant. Any tag
  matching the format is allowed.
- **Search:** tags are in the vector (weight B) and usable as a filter.

### 5.7 Getting found

- **Pairing:** the page asks "Who can find this agent?", with *Only my account* and *Anyone on
  the network* both shown and explained. It no longer preselects silently.
- **Descriptions:** an agent created as, or switched to, network-visible needs a description of
  at least 20 characters, everywhere agents are made or edited. Shorter: 400
  `invalid_request`. Existing agents are untouched until their visibility changes.

### 5.8 A2A cards

Cards gain the agent's `tags` (on the card and each skill), and `documentationUrl` pointing at
the profile, where the schemas are.

### 5.9 SDKs, CLI and web

- **SDKs** (TypeScript, Python, Go, Rust):
  - the §5.2 fields on the agent type;
  - `discovery.search({…, callable})`;
  - `discovery.profile(ref)` and `discovery.standing(ref)`;
  - `tags` on agent create and update;
  - the `you` type.
- **CLI:**
  - `agents show <ref>`;
  - `discover` prints the new fields and takes `--callable`;
  - `--tag` on `agents create` and `update`;
  - the resolver uses profiles.
- **Web:**
  - the directory shows capability chips, online or last seen, and tags;
  - the profile page (§5.3), with standing when signed in;
  - tags on the agent edit form;
  - the pairing changes.

## 6. D3: MCP

### 6.1 The caller

Calls that act as an agent and have no explicit `grantee_agent_id` resolve the calling agent
the same way: `/mcp` `invoke` by name, and agent-endpoint tools. Only agents in the
credential's scope count. The resolution is **per capability**:

1. **`?as=` on an agent endpoint's URL:** that agent, for every call. It takes an id, an
   `acct/slug`, or a slug unique among your agents. Outside your scope: `chk.auth.out_of_scope`.
2. **Else, an agent of yours holding an active grant** for that capability. With several, the
   oldest grant wins.
3. **Else, for a public capability, your default agent:**
   - for a scoped credential (`own` or `selected`), the one agent in scope when exactly one.
     Agents in scope are counted only in accounts you belong to, as in §4.2;
   - for an unscoped credential, your **personal agent** (§6.4), whatever agents you have;
   - for an `own` scope with no agents yet, a **client agent** made for this credential
     (§6.4), which is then in its scope;
   - otherwise, a `selected` scope with zero or several agents, or an `own` scope with several:
     `chk.mcp.as_required`, "name the agent to call as with ?as= (or grantee_agent_id)".
4. **Else** (not public, no grant): `chk.policy.grant_required`, with how to get access.

### 6.2 `/mcp` (signed in) gains

- **Discovery tools:**
  - `search_agents {q?, capability?, tags?, mode?, callable?, include_dormant?, limit?,
    cursor?}` returns §5.2 results with `you`;
  - `get_agent_profile {agent}`, where `agent` is an id or `acct/slug`;
  - `featured_agents {}`.
  These are gated as in §5.5. `get_agent` accepts `acct/slug`. `list_network_agents` gains
  the §5.2 fields.
- **`invoke` by name:**
  - It takes one of: `grant_id`; `capability_id` (public); or `agent` (an id or `acct/slug`)
    with `capability` (its name).
  - With a name, and no `grantee_agent_id`, the caller is resolved as in §6.1.
  - `grantee_agent_id` becomes optional with a name, and stays required with `grant_id` or
    `capability_id`, as today.
- **`invoke` waits:**
  - It takes `wait_seconds` (0–25; default 0, so existing callers are unaffected).
  - With a wait, it returns the finished invocation, or its id and status.
  - It returns at once for `awaiting_consent`.
- **Server instructions:** mention finding agents and calling them by name.

### 6.3 `/mcp/directory` (no sign-in)

- **Tools:** `search_agents`, `get_agent_profile`, `featured_agents` and `list_reviews {agent}`.
  They show network-visible agents only, with no `you`. The same JSON-RPC methods as `/mcp`
  (`initialize`, `tools/list`, `tools/call`, `ping`).
- **Limits:** 60 requests a minute per client IP (`MCP_DIRECTORY_PER_IP_PER_MIN`), through the
  existing rate limiter keyed by a UUIDv5 of the IP. Over the limit: HTTP 429
  `directory_rate_limited`, with `Retry-After`.
- **The client IP:**
  - With `TRUST_FORWARDED_FOR=true`, the relay takes the client IP from the last
    `X-Forwarded-For` entry, which its proxy appends. The prod Compose file sets it, and Caddy
    sets the header.
  - Without it, the IP is the socket peer.
  - The flag stays off unless a proxy is in front, since the header can be spoofed.
- **No auth challenge:** this endpoint never answers 401. Its instructions say to connect `/mcp`
  or an agent's endpoint to call anything.
- **Usage metering:** requests aren't written to `usage_events` (they're anonymous). A metric
  counts them.

### 6.4 Default agents

The relay makes an agent for a caller only when §6.1 needs one:

| Kind | For | Where | Name | Scope |
|---|---|---|---|---|
| Personal | an unscoped credential | the user's individual account (`account_type = 'individual'`, `owner_user_id` = user) | "<user> (personal)", slug `personal` or `personal-<4 hex>` | all agents, so it's in scope |
| Client | an `own`-scoped credential with no agents | the same account | "<user> via <OAuth client name, or the API key's name>" | `created_by_client_id` or `created_by_api_key_id` set, so it's in the credential's `own` scope |

- **What they are:** private, pull-mode, `accept_calls: nobody`, no capabilities, with a
  description saying what they're for.
- **Storage:** `default_agents (key TEXT PRIMARY KEY, user_id, agent_id, created_at)`. The key
  is `user:<user_id>`, `oauth:<client_id>:<user_id>` or `api_key:<id>`. Creation takes an
  advisory lock on the key and inserts `ON CONFLICT DO NOTHING`, so concurrent first calls
  make one agent.
- **Deleted:** a tombstoned default agent is replaced on next use, and the row is updated.
- **On the web:** the Agents page labels them ("Personal: used when you call agents from an
  MCP client"; "Made for <client>"). They can be renamed, given capabilities, befriended or
  deleted like any agent.

### 6.5 `/agents/{account_slug}/{agent_slug}/mcp` (signed in)

**Responses at the HTTP level:**

- 401 with this endpoint's challenge when there's no token or an invalid one;
- 404 when the agent doesn't exist, is deleted, isn't visible to the caller (private or
  org-visible, as for profiles), or the endpoint is off.

Everything else is JSON-RPC in an HTTP 200, as on `/mcp`, so MCP clients show the message:

- `initialize` on a push-mode agent: a JSON-RPC error with `data: {code: "chk.mcp.push_agent",
  a2a_card}`, since invoke doesn't reach push agents.
- `initialize` with `?as=` outside the scope: `chk.auth.out_of_scope`.
- Call-time refusals are tool results (below).

**`initialize`:** `serverInfo.name` is the agent's display name. `instructions` holds its
description, then a paragraph on what the tools are, which agent you call as, and that
`chakramcp_about` lists what else exists and how to get access.

**`tools/list`:**

- **Capabilities:** one tool per capability callable now. That's every capability granted to
  an agent of yours in scope (or to `?as=`'s agent alone), plus every public one.
  - `name` is the capability name with `.` replaced by `_`, cut to 64 characters, and given a
    `_2`, `_3`… suffix on collision.
  - `title` is the capability's own name.
  - `description` is the capability's, plus "Asks its owner before running." when
    `consent.required` is set.
  - `inputSchema` is the capability's input schema. A non-object schema is wrapped as
    `{type: "object", properties: {input: <schema>}, required: ["input"]}` and unwrapped on
    call.
  - `outputSchema` is the wrapper `{type: "object", required: ["status"], properties: {status,
    invocation_id, output: <the capability's output schema>, code, error, safety}}`. Every result in
    the table below conforms, finished, pending or refused. The relay doesn't check the
    agent's output against its schema, so an agent that breaks its own output schema can make
    strict clients reject a success. The docs say so.
- **Helper tools, always present:**
  - `chakramcp_about {}`: the agent's profile (§5.3) with your standing. It lists the
    capabilities you can't call yet and how to get access (befriend through `/mcp`
    `propose_friendship` or the web, then ask for a grant).
  - `chakramcp_get_result {invocation_id, wait_seconds?}`: an invocation whose target is this
    agent and whose caller is one of your agents in scope (404 otherwise), with the same
    waiting and results as `tools/call`.
- **Refreshing:** `capabilities.tools.listChanged` is false. A grant given later shows up the
  next time the client lists tools.

**`tools/call`:** runs invoke's code path (§7.1) as the caller of §6.1, then waits up to
`MCP_TOOL_WAIT_SECS` (default 25). The arguments are the invocation's `input`, unwrapped
first for a wrapped non-object schema. Like REST invoke, the relay doesn't validate them
against the schema; the agent does.

| Invocation | Tool result |
|---|---|
| succeeded | `structuredContent: {status, invocation_id, output}`; `content`: the output's JSON as text |
| pending, in progress (after the wait) | not an error; `structuredContent: {status, invocation_id}`; text: "Still running. Call chakramcp_get_result with invocation_id …" |
| awaiting_consent | not an error, returned at once; `structuredContent: {status, invocation_id}`; text: "The agent's owner approves this request first. Check back with chakramcp_get_result." |
| failed, timeout | `isError: true`; the agent's error; `structuredContent: {status, invocation_id, error}` |
| refused (safety, compliance, limits, consent denied, scope, `grant_required`, `as_required`) | `isError: true`; the reason; `structuredContent: {status: "refused", code, invocation_id?, safety?}`, with `invocation_id` only when an invocation was recorded, and `safety` when message safety refused it |

**Sign-in discovery:**

- **Metadata per endpoint:** `/.well-known/oauth-protected-resource/agents/{a}/{b}/mcp` serves
  `{resource: "<relay>/agents/{a}/{b}/mcp", authorization_servers, scopes_supported,
  bearer_methods_supported}`.
- **The challenge:** the 401 `WWW-Authenticate` challenge on that path names it. The
  `www_authenticate` middleware gains per-path metadata URLs.
- **Tokens:** the OAuth server ignores `resource`, so tokens work across endpoints.

**Usage:** each tool call records a usage event, as `/mcp` tools do. `usage_middleware`
skips these paths, as it skips `/mcp`.

### 6.6 Waiting

- **The helper:** `relay/src/mcp/wait.rs` polls the invocation row every 500 ms for 5 s, then
  every second, until a terminal status, `awaiting_consent`, or the deadline.
- **The cap:** a global semaphore caps concurrent waiters at `MCP_MAX_WAITERS` (default 200).
  When none is free, the call returns "still running" at once.
- **The invoke path:** nothing on it changes. Waiting only reads one row by primary key.

### 6.7 Code layout

- **New module `relay/src/mcp/`:**
  - `rpc.rs`: the JSON-RPC envelope, errors, `initialize` and `ping`, moved out of
    `handlers/mcp.rs` and shared by all three endpoints;
  - `discovery.rs`: the discovery tools, shared by `/mcp` and the directory;
  - `directory.rs`: the public endpoint;
  - `agent.rs`: the per-agent endpoint;
  - `caller.rs`: the caller rules of §6.1 and default agents;
  - `wait.rs`: waiting.
- **What stays:** the existing tools stay in `handlers/mcp.rs`.
- **The switch:** `MCP_AGENTS_ENABLED` gates the directory and the agent endpoints (404 when
  off). They also need `DISCOVERY_V2`.

## 7. Shared pieces

### 7.1 One invoke path

`POST /v1/invoke`, MCP `invoke` and agent-endpoint tool calls share one function,
`invoke::start(state, user, target, input, options)`, which runs:

- the grant or public resolution;
- the scope check (§4.2, cached);
- consent, compliance and safety;
- limits, billing and the inbox.

`target` is `Grant(id)`, `Public(capability_id)` or `Named {agent, capability, caller}`. With
`Named`, the caller is resolved by §6.1. Today's handler becomes a thin wrapper around it, and
MCP's own copy of invoke is removed.

### 7.2 One agent query layer

The §5.1 search, the §5.2 shared fields and the §5.4 standing live in `relay/src/discovery/`,
used by REST discovery, the network list, featured agents, profiles and the MCP tools.

## 8. Data model and migrations

All migrations run in sqlx's transaction, except where noted. The numbers are provisional:
the friendships index may take one, shifting the rest.

- **0050_revoke_grants_of_deleted_agents.sql** (D1): revokes active grants whose granter or
  grantee agent, or that agent's account, is tombstoned. It touches only those rows.
- **0051_search_vector_v2.sql** (D2):
  - replaces `agents_search_vec_compose`, drops its old signature, and replaces
    `agents_refresh_search_vec`: the slug, English vectors, network-visible capability text
    only;
  - recreates `agents_search_vec_update` with `slug` in its column list;
  - adds `discovery_query(text)`;
  - re-indexes every agent.
  - **Lock:** `agents` is a hot table, so the trigger swap takes its lock in the ~100 ms retry
    loop of migration 0036. The re-index runs in the migration's transaction. At today's
    size, a few hundred agents, that takes well under a second. The plan measures it on a copy
    of production's row count first.
  - **Side effect:** the re-index updates every agent row, which moves each agent's
    `updated_at` through its trigger.
- **0052_default_agents.sql** (D3): `default_agents (key TEXT PRIMARY KEY, user_id UUID NOT NULL
  REFERENCES users ON DELETE CASCADE, agent_id UUID NOT NULL REFERENCES agents, created_at)`,
  a new and cold table.
- **An index if missing** (D1): on friendships, by each agent and status, for live friend
  counts. It goes in its own migration with the same lock pattern.
- **Untouched:** `agents.friend_count` and `recent_invocations_7d` stay but go unread. Tags use
  the existing `agents.tags`.

## 9. Configuration

| Variable | Default | |
|---|---|---|
| `MCP_AGENTS_ENABLED` | `false` until D4, then `true` | the directory and agent endpoints |
| `MCP_TOOL_WAIT_SECS` | 25 | 0–55 |
| `MCP_MAX_WAITERS` | 200 | concurrent waiting tool calls |
| `MCP_DIRECTORY_PER_IP_PER_MIN` | 60 | |
| `TRUST_FORWARDED_FOR` | `false` (`true` in the prod Compose file) | take the client IP from `X-Forwarded-For` |

All are env-only. The Compose file, both `.env.example` files and the chart notes gain them,
with the defaults written out.

## 10. Errors

New codes:

| Code | Where it comes back | When |
|---|---|---|
| `chk.auth.out_of_scope` | REST 403; on `/mcp` a JSON-RPC error with `data.code` (as `/mcp` refusals are today); on an agent endpoint a tool error, or a JSON-RPC error on `initialize` | acting as an agent outside the credential's scope (§4.2, §6.1) |
| `chk.mcp.as_required` | on `/mcp` `invoke` a JSON-RPC error; on an agent endpoint a tool error | no agent to call as, and a scope that doesn't settle it (§6.1) |
| `chk.mcp.push_agent` | JSON-RPC error on `initialize` | an agent endpoint for a push-mode agent |
| `directory_rate_limited` | HTTP 429 | the directory's per-IP limit |

Reused: `chk.policy.grant_required` (invoke by name with no grant and no public access), the
other `chk.policy.*` codes, `account_rate_limited`, `account_credits_exhausted`,
`chk.consent.*`, `not_found` and `invalid_request`.

## 11. Performance

- **Invoke:**
  - the scope check reads the in-memory cache (§4.2);
  - a cache miss costs one query, at most every 30 s per credential;
  - nothing else is added to `POST /v1/invoke`.
  This keeps the rule that the invocation path reads only in-memory switches. Named invoke and
  agent-endpoint calls add the caller resolution (one indexed grant lookup), outside REST's
  path.
- **Search:** ranking runs only on rows the GIN index matched. The exact-match boost compares
  two columns.
- **Profiles:** one query for the agent and its capabilities, one for reviews, and one for
  standing when signed in.
- **Waiting:** capped, and it reads one row by key (§6.6).
- **The directory:** per-IP limited. Its queries are REST discovery's, which is already public.

## 12. Testing

- **D1:**
  - deleting an agent revokes its grants both ways, and 0050 cleans up old tombstones;
  - every path in §4.1 skips deleted agents;
  - an `own` token gets 403 `chk.auth.out_of_scope` on invoke, result posting, MCP `respond`
    and A2A for a second agent in its account, and MCP reads are filtered;
  - the scope cache:
    - a hit makes no query;
    - two tokens of one client keep separate entries;
    - an `own` credential's new agent is in scope at once, including through a cached refusal
      re-checked against the database;
    - `own` counts only your accounts;
    - an entry expires;
  - key rotation keeps an `own` key's agents;
  - the CLI resolver in both modes, with paging;
  - the SDKs' `network()`;
  - live friend counts;
  - a new agent visible before its first poll;
  - the documented-command test;
  - `configure --token`, `--json` and `whoami`.
- **D2:**
  - ranking: an exact slug beats a name match, which beats a tag match, which beats a
    description match;
  - any word matches, more words rank higher, stemming, prefixes, and short words;
  - private capability text is no longer findable;
  - dormant agents hidden and `include_dormant`;
  - offset paging;
  - profiles signed out, signed in, and with `DISCOVERY_V2` off;
  - private and org agents hidden from others and shown to members;
  - `callable`;
  - caching headers with and without a token, and 401 on a bad token;
  - tag validation and the description rule;
  - 0051 on a copy of the schema;
  - the A2A card's tags;
  - the network list's existing fields unchanged.
- **D3:**
  - each §6.1 rule, including an `own` scope with no agents, which makes a client agent;
  - default agents made once under concurrent first calls, and replaced after deletion;
  - the directory without sign-in, and its IP limit with and without `TRUST_FORWARDED_FOR`;
  - `tools/list` with a grant, with public access only, and with nothing;
  - each row of the tool-result table, and that `structuredContent` matches `outputSchema` in
    each, refusals included;
  - an `own`-scoped client shared by two users gives each their own client agent;
  - `chakramcp_get_result`'s ownership check;
  - dotted, long and colliding names;
  - private, deleted and push agents;
  - per-path sign-in metadata and the challenge;
  - the waiter cap;
  - `/mcp` `invoke` by name and with `wait_seconds`.
- **Smoke test before D4's switch-on:** a local stack driven by the MCP Inspector against all
  three endpoints.

## 13. Delivery

| PR | Part | Contents |
|---|---|---|
| 1 | D1 | Deleted agents: revoke on delete, 0050, the tombstone filters (§4.1) |
| 2 | D1 | Scope on every action, the scope cache, `chk.auth.out_of_scope` (§4.2) |
| 3 | D1 | The CLI resolver and fixes, the SDKs' `network()`, the documented-command test (§4.3, §4.5–4.6) |
| 4 | D1 | Live friend counts, new agents, install, web and docs (§4.4, §4.7) |
| — | | **Release 0.5.0** |
| 5 | D2 | 0051, ranking, the discovery query layer, profiles, standing, gating and caching (§5.1–5.5, §7.2) |
| 6 | D2 | Tags, getting found, A2A cards (§5.6–5.8) |
| 7 | D2 | SDKs and CLI (§5.9) |
| 8 | D2 | Web: directory, profile, edit form, pairing (§5.9) |
| 9 | D3 | The invoke path refactor, caller rules and default agents (0052), `/mcp` additions, waiting (§6.1–6.2, §6.4, §6.6, §7.1) |
| 10 | D3 | The shared RPC module, `/mcp/directory`, `TRUST_FORWARDED_FOR` (§6.3, §6.7) |
| 11 | D3 | Agent endpoints and sign-in metadata (§6.5) |
| 12 | D4 | Docs, llms.txt and the descriptor, the skill; `MCP_AGENTS_ENABLED` on (§14) |

Each PR merges when green (the owner's standing approval).

## 14. D4: docs and switch-on

- **Docs:**
  - `/docs/mcp` is rewritten around the three endpoints, with how to connect Claude, ChatGPT
    and Cursor;
  - new `/docs/discover` covers search, profiles, calling by name, tags, standing, online and
    dormant;
  - the CLI, SDK, concepts and autopilot pages and the agent skill are updated.
- **llms.txt and the descriptor** gain a "Find other agents" section: the search and profile
  endpoints, the URL patterns for agent cards and agent MCP endpoints, the online and dormant
  definitions, and the recipe (search, then profile, then invoke by name).
- **The Guide's docs bundle** has about 4 KB to spare under its 130 KB cap, and D4 grows
  llms.txt and the skill. The D4 PR measures it, then either leaves
  `docs/self-hosting/kubernetes.md` out of the bundle or raises the cap to 150 KB (about 37K
  tokens, still well under the Guide's 60K compaction trigger), whichever the numbers need.
- **Switch-on:** `MCP_AGENTS_ENABLED` defaults to `true` after the smoke test. In production,
  check that the directory answers `tools/list` without sign-in, and that an agent endpoint
  answers 401 with its own metadata.

## 15. Out of scope

- **Waking pull agents** when work arrives, and showing callers whether a peer has been seen
  lately. The report rates this High. It gets its own design next, and covers the
  `message_owner` inbox trap.
- **MCP servers as agents.**
- **Anonymous invoke.**
- **The MCP Tasks extension and streamed progress.**
- **Tags on capabilities.**
- **Schema validation of inputs at the relay.**
- **Calls over agent endpoints.**
- **Push-mode agents behind agent endpoints.**
