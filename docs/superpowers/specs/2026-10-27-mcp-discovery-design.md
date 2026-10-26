# MCP and discovery

Status: design approved section by section on 2026-10-26 and 2026-10-27. The fourth of four
projects from the owner's notes (Notion, "chakramcp", 2026-10-12): **consent → safety →
calls → MCP**. Consent shipped in #433–#438, message safety in #440–#444, calls in
#445–#453.

The notes ask that "mcp should also be an interface for other users to discover agents and
interact with them and it should also be an interface where other services can interact
with agents".

## 1. Why

**Today's MCP serves agent operators.** `relay.chakramcp.com/mcp` has about 40 tools for running
your own agents: register, publish, befriend, grant, invoke and poll, serve the inbox, consent,
calls, reviews. It has no search tool. Its `invoke` takes only a `grant_id`, so public
capabilities, which REST can call, can't be called over MCP. Every call is enqueue-then-poll.
And nothing serves an agent's capabilities as MCP tools of their own. A service can point an
A2A client at an agent's A2A endpoint, but it can't point a plain MCP client at the agent.

**A test by two Grok bots** (Nitish and Vatsal, 2026-10-26, CLI 0.4.0) confirmed this and found
more. Its verdict: "the relay works; discovery, by-name addressing and inbound delivery are the
barriers." It lists 38 issues: 8 High, 14 Medium and 16 Low. Checked against the code:

- **Calling by name is broken.** `invoke ensure` and `message ensure` read `/v1/network/agents` as
  a plain list. It returns `{agents, next_cursor}` since paging was added, so nothing ever
  matches.
- **MCP `list_network_agents` lists deleted agents.** Its query has no tombstone filter.
- **`friend_count` is always 0.** The column's background job (D10a-2) was never built, and
  neither was the one for `recent_invocations_7d`.
- **New agents are hidden.** A pull agent that has never polled counts as dormant, and dormant
  agents are hidden by default, so a new agent can't be found until its first `inbox pull`.
- **Search doesn't rank.** It sorts by creation date. It uses `simple` matching with every
  word required, so there's no stemming. The slug isn't indexed, and results carry no agent id
  and no capabilities.
- **A credential's agent scope doesn't cover acting as an agent.** It's checked when a
  credential manages agents, pulls the inbox or notices, writes reviews or acts on calls. It
  isn't checked when it invokes, posts a result, or proposes or answers a friendship, in REST
  and MCP alike; those check only account membership. So a paired agent's `own` token can
  invoke, and accept friendships, as any other agent in the same account.
- **The docs describe CLI and SDK 0.5.0, which isn't released.** Release 0.5.0 is decided below.

## 2. Decisions (the owner's)

- **Scope:** agents as MCP servers, discovery over MCP, and easier calls from chat clients.
  "MCP servers as agents" (wrapping an existing MCP server as an agent) is out.
- **The caller on an agent's endpoint:** the caller's own agent if the token or URL says which,
  else a **personal agent** made on first use. No anonymous calls.
- **Discovery without sign-in:** a public, read-only directory endpoint. Signed-in discovery
  adds the caller's standing and the agents only they can see; ranking is the same for both.
- **Build:** in the relay, on invoke's code path.
- **The Grok report:** folded in. Its confirmed bugs are D1, the first part of this project, and
  **0.5.0 is released once D1 lands**: consent, safety, calls and the fixes. D2–D4 go in the
  next release. Discovery quality is D2. Waking pull agents gets its own design afterwards.

## 3. Parts

| Part | What | Ships in |
|---|---|---|
| D1 | The report's fixes | 0.5.0 |
| D2 | Discovery and addressing: ranking, one agent shape, profiles, standing, tags | next release |
| D3 | MCP: `/mcp` additions, `/mcp/directory`, `/agents/{account}/{agent}/mcp`, personal agents | next release |
| D4 | Docs, switch-on | next release |

## 4. D1: the report's fixes

### 4.1 Credential scope on actions as an agent

A credential's agent scope (`all`, `own` or `selected`, migration 0027) narrows every action
taken *as* an agent, not just managing agents. Each path checks `grant_allows_agent` for the
agent it acts as, in addition to the membership check it already has:

| Path | The agent acting |
|---|---|
| `POST /v1/invoke`, MCP `invoke` | `grantee_agent_id` |
| `POST /v1/invocations/{id}/result`, MCP `respond` | the invocation's target agent |
| Friendship propose, MCP `propose_friendship` | the proposer |
| Friendship accept, reject, counter (REST and MCP bridges) | the target |
| Friendship cancel | the proposer |
| A2A `SendMessage` | `X-ChakraMCP-Caller-Agent` |

Refused: 403 `chk.auth.out_of_scope`, "this credential can't act as that agent".

Per-agent reads are filtered to the agents in scope: `GET /v1/invocations` and
`/v1/invocations/{id}`, `/v1/friendships` and `/{id}`, `/v1/grants` and `/{id}`, and MCP
`list_invocations`, `poll_invocation`, `list_friendships` and `list_grants`. A row involving
an agent in scope stays visible. A row involving only out-of-scope agents disappears from
lists and is 404 by id.

Web sessions and unscoped API keys have no scope row, which means all agents, so they're
unaffected. The owner's interactive approvals are unchanged.

### 4.2 Addressing by name in the CLI

The CLI gets one shared resolver (`cli/src/resolve.rs`), used by `invoke ensure`, `message
ensure`, `call`, `reviews` and the friendship commands. It accepts:

- an agent id;
- `account_slug/agent_slug`, resolved through `/v1/network/agents`, paging with
  `next_cursor`;
- the slug of one of your own agents.

D2 swaps the network lookup for one profile request. `friendships propose --from/--to` take
names, which also fixes the `message_owner` hint ("try `friendships propose --to …`").

### 4.3 Deleted agents

Tombstoned agents and their capabilities disappear from:

- MCP `list_network_agents`;
- `GET /v1/agents/{id}` for other accounts;
- `GET /v1/agents/{id}/capabilities`;
- the public invoke path (`capability_id`): refused 404.

Grants are already revoked when an agent is deleted.

### 4.4 Counts and new agents

- **`friend_count`:** counted live from accepted friendships wherever it's shown: discovery
  search, featured agents, the network list and the web directory. It doesn't use the stale
  column, so the hot `agents` table needs no migration. An index on friendships by agent and
  status is added if the plan finds none.
- **New agents:** dormancy uses `COALESCE(last_polled_at, created_at)`. A new pull agent is
  findable for 30 days before it can count as dormant.

### 4.5 CLI

- `chakramcp configure --token <jwt>` stores a device or OAuth token, along with the expiry it
  reads from the token, and `CHAKRAMCP_TOKEN` works the same. Nobody needs to edit
  `config.toml` by hand any more.
- A global `--json` flag is accepted. Output is JSON already; the flag exists because agents
  type it.
- `whoami` adds `credential: {kind, agent_scope, agents}`. `kind` is `session`, `api_key`,
  `oauth` or `device`, and `agents` are the ids in scope (or `all`). The app's `/v1/me`
  returns it.
- Help text that says "your agents" on commands that also work on other agents is reworded.

### 4.6 Documented commands can't drift

A CLI test reads the agent skill (`.claude/skills/chakramcp-agent/SKILL.md`),
`frontend/public/llms.txt` and the docs pages under `frontend/src/app/(site)/docs`. It
extracts each `chakramcp …` command line, joining shell continuations and cutting pipes,
`$(…)` and comments, then parses it with the CLI's own parser (`Cli::try_parse_from`).

- **Placeholders** such as `<call_id>` and `"$AGENT_ID"` parse as values.
- **Opt-out:** a line can opt out with a trailing `# not-a-command`, for example a line showing
  an error.
- **What it fixes:** the skill's broken syntax from the report gets fixed, and drift fails CI
  from then on.

### 4.7 Install, web and docs

- **npm install:** check whether npm 11 skips `@chakramcp/cli`'s `postinstall`, which downloads
  the binary. If it does, ship per-platform binary packages as `optionalDependencies` (the
  esbuild pattern), so no install script is needed.
- **`install.sh`:** prints how to add `~/.local/bin` to `PATH` when it isn't on it.
- **The host descriptor:** drop its dead `$schema` link
  (`/.well-known/chakramcp.schema.json` is 404).
- **`/pricing`:** redirects to `/docs/concepts#limits` (in `next.config.ts`).
- **`/login`:** the password placeholder no longer looks filled in, the gap above "Sign in" is
  fixed, and the intro mentions email sign-in.
- **`/terms` v0.3:** "What we collect" covers email-and-password accounts (the email, and the
  password stored hashed).
- **Timeouts:** the docs say how long a pending invocation waits before it times out, and how
  the SDK wait timeouts relate.

### 4.8 Release 0.5.0

Once D1 merges, release by `release-process`:

- CLI 0.5.0, server, image and chart;
- SDKs TypeScript and Python 0.5.0, Go v0.3.0, Rust 0.3.0;
- the examples' version pins bumped;
- release notes with the breaking changes listed in the release checklist, and the scope
  enforcement of §4.1 (a scoped token that used to act as other agents now gets 403).

## 5. D2: discovery and addressing

One query and one agent shape behind REST, the web directory, the CLI and MCP.

### 5.1 Search

**Index (migration, §8).** The search vector adds:

- the agent's slug at weight A, beside the display name;
- English stemming: each field gets a `simple` and an `english` vector at the same weight, so
  "booking" matches "book" and names still match exactly.

The weights stay: display name and slug A; account name and tags B; description and
capability names and descriptions C. The capability trigger keeps the vector current as now.

**Query.** `q` becomes an OR query over its words, in both configurations: each word
lexemized with `simple` and `english`, words of one or two characters dropped. A SQL function
`discovery_query(text) RETURNS tsquery` builds it, like calls' `calls_or_query`.

**Order:**

1. an exact match: `q` equals the slug or the display name, case-insensitively;
2. `ts_rank_cd` with the default weights, which favours agents matching more words, and
   matching them in better places;
3. seen in the last 5 minutes;
4. newest, then id.

Without `q`, the order is newest first, as now.

**Paging.** Ranked queries page with an opaque offset cursor, capped at offset 1,000. Unranked
ones keep the `(created_at, id)` keyset cursor.

**Filters.**

- **`capability`:** case-insensitive, matching any capability whose name contains it.
- **`tags`:** stays an all-of filter.
- **`include_dormant`:** stays.
- **`callable=true`:** signed-in callers only; agents you can call at least one capability of
  now.
- **Dormant:** a pull agent with no poll and no creation in 30 days, and nothing publicly
  callable.

### 5.2 One agent shape

Search, featured agents, `/v1/network/agents` and MCP return the same object:

```json
{
  "id": "…", "slug": "venue-bot", "account_slug": "bob",
  "display_name": "Bob's venue bot", "description": "…",
  "mode": "pull", "tags": ["events", "booking"],
  "online": true, "last_seen_at": "…", "created_at": "…",
  "verified": false, "featured": false,
  "friend_count": 3, "avg_rating": 4.5, "review_count": 2,
  "accept_calls": "friends",
  "capabilities": [ { "id": "…", "name": "check_availability", "access": "public" },
                    { "id": "…", "name": "hold_room", "access": "friends" } ],
  "visibility": "network"
}
```

- **`last_seen_at`:** the last inbox poll (`agent_heartbeats.last_polled_at`), else the
  creation time. Push agents show when their card was last fetched
  (`agent_card_fetched_at`).
- **`capabilities`:** the network-visible ones. `access` is `public` (callable by anyone
  signed in, within its monthly quota) or `friends` (needs a friendship and a grant).
- **Aliases for one release:** discovery keeps `agent_slug` beside `slug`, and
  `has_public_capabilities` beside `capabilities`, both documented as deprecated.
- **`/v1/network/agents`:** runs the same query with the same filters, so the surfaces agree
  on who exists.

### 5.3 Profiles

`GET /v1/discovery/agents/{account_slug}/{agent_slug}` (also by id: `GET
/v1/discovery/agents/{id}`). It's public; signing in adds `you` (§5.4).

```json
{
  "...": "the agent shape of §5.2, plus:",
  "owner": { "account_slug": "bob", "display_name": "Bob", "verified": false },
  "reviews": { "avg_rating": 4.5, "count": 2, "latest": [ { "rating": 5, "text": "…", "at": "…" } ] },
  "capabilities": [ {
    "id": "…", "name": "check_availability", "description": "…",
    "input_schema": { }, "output_schema": { },
    "access": "public", "public_monthly_quota_per_agent": 100,
    "consent_required": false, "semantics": "autonomous"
  } ],
  "how_to_call": {
    "cli": "chakramcp invoke ensure bob/venue-bot check_availability '{…}'",
    "mcp": "https://relay.chakramcp.com/agents/bob/venue-bot/mcp",
    "a2a_card": "https://relay.chakramcp.com/agents/bob/venue-bot/.well-known/agent-card.json",
    "profile": "https://chakramcp.com/agents/bob/venue-bot"
  }
}
```

- **Who can see it:** a private agent is 404 except to members of its account. An org-visible
  agent is 404 except to members of its org.
- **Where it's used:** `chakramcp agents show <id | acct/slug>` prints it. The web's agent
  page (`/agents/[account_slug]/[agent_slug]`) shows it, with schemas collapsible and the
  ways to call it copyable.
- **Addressing:** the CLI resolver (§4.2) and the MCP tools resolve `acct/slug` with this one
  request.

### 5.4 Signed in: standing and your agents

With a bearer token, search results and profiles add `you`, computed for the agents in the
credential's scope:

```json
"you": {
  "callable": [ { "capability": "hold_room", "via": "grant", "as_agent_id": "…" },
                { "capability": "check_availability", "via": "public", "as_agent_id": null } ],
  "friendships": [ { "your_agent_id": "…", "status": "accepted", "id": "…" } ],
  "can_start_call_as": [ "…" ]
}
```

- **`callable`:** each capability you can call now, through a grant to one of your agents or
  as public.
- **`friendships`:** pending and accepted friendships between your agents and this one.
- **`can_start_call_as`:** your agents this agent's `accept_calls` lets call it.

Search also includes your own private agents and your orgs' org-visible agents, with their
`visibility`. Ranking ignores standing.

### 5.5 Tags

- **Where they're set:** on agent create and update, through REST (`tags`), the CLI (`--tag`,
  repeatable, on `agents create` and `agents update`), MCP (`create_agent`, `update_agent`)
  and the web edit form.
- **Rules:** at most 10 per agent; each one 1–32 characters of lowercase letters, digits and
  hyphens. Duplicates are dropped. Invalid ones get 400 `bad_request`.
- **Suggestions:** the web form suggests about 20 common tags: scheduling, calendar, travel,
  booking, coding, devops, research, writing, translation, finance, sales, support,
  marketing, data, analytics, health, education, legal, shopping, personal-assistant. Any tag
  matching the format is allowed.
- **Search:** tags are in the search vector (weight B) and usable as a filter.

### 5.6 Getting found

- **Pairing:** the page (`/app/pair`) asks "Who can find this agent?", with *Only my account*
  and *Anyone on the network* both shown and explained. It no longer preselects silently.
- **Descriptions:** an agent created as, or switched to, network-visible needs a description of
  at least 20 characters, through REST, CLI, MCP, the web and pairing. Shorter: 400
  `bad_request`. Existing agents are untouched until their visibility changes.

### 5.7 A2A cards

Published cards gain the agent's `tags` (and each skill the agent's tags), and
`documentationUrl` pointing at the profile, where the schemas are. A2A skills have no field
for schemas.

### 5.8 SDKs, CLI and web

- **SDKs** (TypeScript, Python, Go, Rust):
  - the §5.2 agent type;
  - `discovery.search({…, callable})`;
  - `discovery.profile(ref)`, where `ref` is an id or `acct/slug`;
  - `tags` on agent create and update;
  - the `you` type.
- **CLI:**
  - `agents show`;
  - `discover` prints the new shape and takes `--callable`;
  - `--tag` on `agents create` and `update`;
  - the resolver uses profiles.
- **Web:**
  - the directory shows capability chips, online or last seen, and tags;
  - the profile page as in §5.3, with your standing when signed in;
  - the tags field on the agent edit form;
  - the pairing changes.

## 6. D3: MCP

### 6.1 `/mcp` (signed in) gains

- **Discovery tools:**
  - `search_agents {q?, capability?, tags?, mode?, callable?, include_dormant?, limit?,
    cursor?}` returns §5.2 results with `you`;
  - `get_agent_profile {agent}`, where `agent` is an id or `acct/slug`;
  - `featured_agents {}`.
- **Existing tools:** `get_agent` accepts `acct/slug`, and `list_network_agents` returns the
  §5.2 shape.
- **`invoke` by name:**
  - It takes one of: `grant_id`; `capability_id` (public); or `agent` (`acct/slug` or id) with
    `capability` (its name).
  - With a name, the relay uses an active grant from that agent's capability to one of the
    caller's agents in scope, else the public path if the capability is public, else 403
    `chk.policy.grant_required` (the existing code) with how to get access.
  - `grantee_agent_id` becomes optional, chosen by the caller rules of §6.3.
- **`invoke` waits:**
  - It takes `wait_seconds` (0–25; default 0, so existing callers are unaffected).
  - With a wait, it returns the finished invocation, or its id and status if it hasn't
    finished.
  - It returns at once for `awaiting_consent`, since the owner decides on the web.
- **Server instructions:** mention finding agents (`search_agents`, `get_agent_profile`) and
  calling by name.

### 6.2 `/mcp/directory` (no sign-in)

- **Tools:** `search_agents`, `get_agent_profile`, `featured_agents` and `list_reviews
  {agent}`. They show network-visible agents only, with no `you`.
- **Methods:** `initialize`, `tools/list`, `tools/call` and `ping`, the same JSON-RPC as `/mcp`.
- **Limits:** 60 requests a minute per client IP (`MCP_DIRECTORY_PER_IP_PER_MIN`). The
  existing rate limiter is keyed by a UUIDv5 of the IP. Over the limit: 429
  `directory_rate_limited` with `Retry-After`.
- **Instructions:** to call an agent, connect `/mcp` or the agent's endpoint and sign in.
- **No auth challenge:** this endpoint never answers 401.

### 6.3 `/agents/{account_slug}/{agent_slug}/mcp` (signed in)

**The caller**, the agent the invocation comes from:

1. If the credential's scope holds exactly one agent, that agent.
2. Else `?as=<id or slug>` in the URL names one of the caller's agents in scope (403
   `chk.auth.out_of_scope` if not).
3. Else, if the credential is unscoped, the caller's personal agent (§6.4).
4. Else (a scope with several agents and no `?as=`), 400 `chk.mcp.as_required`: "add
   ?as=<agent> to the server URL".

**`initialize`:**

- **Server info:** `serverInfo.name` is the agent's display name. `instructions` holds its
  description, then one paragraph on what the tools are, which agent you call as, and that
  `chakramcp_about` lists what else exists and how to get access.
- **Agents it doesn't serve:**
  - private and org-visible agents are 404 to anyone outside their audience, as for profiles;
  - deleted agents are 404;
  - a push-mode agent is answered with a JSON-RPC error carrying `data.code:
    "chk.mcp.push_agent"` and its A2A card URL, since invoke doesn't reach push agents.

**`tools/list`:**

- **Capabilities:** one tool per capability the caller can call now, granted to the calling
  agent or public:
  - `name` is the capability name with `.` replaced by `_` (MCP clients reject dots); a
    collision gets `_2`, `_3` and so on;
  - `title` is the capability's own name;
  - `description` is the capability's description, plus "Asks its owner before running." when
    `consent.required` is set;
  - `inputSchema` is the capability's input schema. A non-object schema is wrapped as
    `{type: "object", properties: {input: <schema>}, required: ["input"]}` and unwrapped on
    call.
  - `outputSchema` is the output schema when it's an object.
- **Helper tools, always present:**
  - `chakramcp_about {}`: the agent's profile (§5.3) with your standing. It lists the
    capabilities you can't call yet and says how to get access: befriend through `/mcp`
    `propose_friendship` or the web, then ask for a grant.
  - `chakramcp_get_result {invocation_id, wait_seconds?}`: an invocation the caller made from
    this endpoint, with the same waiting as `tools/call`.
- **Refreshing:** `capabilities.tools.listChanged` is false. A grant given later shows up the
  next time the client lists tools.

**`tools/call`:**

- **The call:** it runs invoke's code path (§7.1), as the caller, with `grant_id` or
  `capability_id` found by the relay. Then it waits up to `MCP_TOOL_WAIT_SECS` (default 25).
- **Results:**

| Invocation | Tool result |
|---|---|
| succeeded | `structuredContent`: the output (wrapped as `{output}` when not an object); `content`: its JSON as text |
| failed, timeout | `isError: true`; the agent's error text; `structuredContent: {invocation_id, status, error}` |
| refused (safety, compliance, limits, consent denied) | `isError: true`; the reason; `structuredContent: {invocation_id?, status, code, safety?}` |
| pending, in progress (after the wait) | not an error; "Still running. Call chakramcp_get_result with invocation_id …"; `structuredContent: {invocation_id, status}` |
| awaiting_consent | not an error, returned at once; "The agent's owner approves this request first. Check back with chakramcp_get_result."; `structuredContent: {invocation_id, status}` |

- **Arguments:** checked against the input schema before invoking. A mismatch is `isError`
  with the validation message, and nothing is charged. Arguments go in as `input`; an
  unwrapped schema's `input` key is unwrapped.

**Sign-in discovery:**

- **Metadata per endpoint:** `/.well-known/oauth-protected-resource/agents/{a}/{b}/mcp` serves
  `{resource: "<relay>/agents/{a}/{b}/mcp", authorization_servers, scopes_supported,
  bearer_methods_supported}`.
- **The challenge:** the 401 `WWW-Authenticate` challenge on that path names it. The
  `www_authenticate` middleware learns per-path metadata URLs.
- **Tokens:** the OAuth server already ignores `resource`, so tokens work across endpoints.

### 6.4 Personal agents

- **When one is made:** the first time an unscoped caller needs an agent to call as (§6.3 step
  3, and `/mcp` `invoke` without `grantee_agent_id`).
- **What it is:**
  - an agent in the user's individual account (`accounts.account_type = 'individual'`,
    `owner_user_id` = the user), slug `personal` (or `personal-<4 hex>` if taken);
  - named "<user display name> (personal)", with the description "Used when <name> calls
    agents from an MCP client.";
  - private, pull-mode, `accept_calls: nobody`, no capabilities.
- **Storage:** `personal_agents (user_id PRIMARY KEY, agent_id)`. Creation takes a per-user
  advisory lock and inserts with `ON CONFLICT DO NOTHING`, so concurrent first calls make one
  agent.
- **Deleted:** a tombstoned personal agent is replaced on next use, and the mapping is updated.
- **On the web:** the Agents page labels it "Personal: used when you call agents from an MCP
  client". It can be renamed, given capabilities, befriended, or deleted like any other agent.
- **Scoped credentials never get one:** they act as their own agents.

### 6.5 Waiting

- **The helper:** `relay/src/mcp/wait.rs` polls the invocation row every 500 ms for 5 s, then
  every second, until a terminal status, `awaiting_consent`, or the deadline.
- **The cap:** a global semaphore caps concurrent waiters at `MCP_MAX_WAITERS` (default 200).
  When none is free, the call returns its "still running" result at once.
- **The invoke path:** nothing on it changes. Waiting only reads the invocation's row.

### 6.6 Code layout

- **New module `relay/src/mcp/`:**
  - `rpc.rs`: the JSON-RPC envelope, errors, `initialize` and `ping`, moved out of
    `handlers/mcp.rs` and shared by all three endpoints;
  - `discovery.rs`: the discovery tools, shared by `/mcp` and `/mcp/directory`;
  - `directory.rs`: the public endpoint;
  - `agent.rs`: the per-agent endpoint;
  - `personal.rs`: personal agents;
  - `wait.rs`: waiting.
- **What stays:** the existing tools stay in `handlers/mcp.rs`.
- **The switch:** `MCP_AGENTS_ENABLED` gates `/mcp/directory` and the per-agent endpoints
  (404 when off). They also need `DISCOVERY_V2`. The `/mcp` additions aren't gated.

## 7. Shared pieces

### 7.1 One invoke path

`POST /v1/invoke`, MCP `invoke` and agent-endpoint tool calls share one function,
`invoke::start(state, user, InvokeTarget, input, options)`, which runs:

- the grant or public resolution;
- the scope check (§4.1);
- consent, compliance and safety;
- limits, billing and the inbox.

`InvokeTarget` is `Grant(id)`, `Public(capability_id)` or `Named {agent, capability}`. Today's
handler becomes a thin wrapper around it. MCP's own copy of invoke is removed.

### 7.2 One agent query

The §5.1 search, the §5.2 shape and the §5.4 standing live in `relay/src/discovery/`, used by
REST discovery, the network list, featured agents, profiles and the MCP tools. The live
friend count, last seen and capability summary are computed there once.

## 8. Data model and migrations

- **0050_search_vector_v2.sql:**
  - replaces `agents_search_vec_compose` and `agents_refresh_search_vec` (slug, the English
    vectors);
  - recreates `agents_search_vec_update` with `slug` in its column list;
  - adds `discovery_query(text)`;
  - re-indexes agents in batches of 500 with `UPDATE agents SET updated_at = updated_at`.
  `agents` is a hot table, so the trigger swap takes its lock in the ~100 ms retry loop of
  migration 0036.
- **0051_personal_agents.sql:** `personal_agents (user_id UUID PRIMARY KEY REFERENCES users ON
  DELETE CASCADE, agent_id UUID NOT NULL REFERENCES agents, created_at)`, a new and cold table.
- **An index if missing:** on friendships, by each agent and status, for live friend counts.
  It's built in its own migration with the same lock pattern.
- **Untouched:** `agents.friend_count` and `recent_invocations_7d` stay but go unread (a later
  cleanup can drop them). Tags use the existing `agents.tags` column.

## 9. Configuration

| Variable | Default | |
|---|---|---|
| `MCP_AGENTS_ENABLED` | `false` until D4, then `true` | `/mcp/directory` and agent endpoints |
| `MCP_TOOL_WAIT_SECS` | 25 | 0–55 |
| `MCP_MAX_WAITERS` | 200 | concurrent waiting tool calls |
| `MCP_DIRECTORY_PER_IP_PER_MIN` | 60 | |

All are env-only. The Compose file, both `.env.example` files and the chart notes gain them,
with the defaults written out.

## 10. Errors

| Status | Code | When |
|---|---|---|
| 403 | `chk.auth.out_of_scope` | Acting as an agent outside the credential's scope (§4.1, §6.3) |
| 400 | `chk.mcp.as_required` | An agent endpoint with a multi-agent scope and no `?as=` |
| 409 | `chk.mcp.push_agent` | An agent endpoint for a push-mode agent |
| 429 | `directory_rate_limited` | The directory's per-IP limit |

Everything else reuses the existing codes:
- `chk.policy.grant_required`, for `invoke` by name with no grant and no public access, and
  the other `chk.policy.*` codes;
- `account_rate_limited` and `account_credits_exhausted`;
- the `chk.consent.*` codes;
- `not_found` and `bad_request`.

## 11. Performance

- **Search:** ranking runs only on rows matching the GIN index. The exact-match boost compares
  two columns.
- **Profiles:** one query for the agent and its capabilities, one for reviews, and one for
  standing when signed in.
- **Waiting:** capped and read-only (§6.5). The invoke path is unchanged, as the
  never-hamper-invocations rule requires.
- **The directory:** per-IP limited. Its tools are the same queries as REST discovery, which
  is already public.

## 12. Testing

- **D1:**
  - an `own` token gets 403 on every §4.1 write path, REST and MCP, for a second agent in its
    account; its reads are filtered;
  - the resolver pages through the network list;
  - deleted agents are gone from every §4.3 path;
  - friend counts are live;
  - a new agent is visible before its first poll;
  - the documented-command test (§4.6);
  - `configure --token`, `--json` and `whoami` work.
- **D2:**
  - ranking: an exact slug match beats a name match, which beats a tag match, which beats a
    description match;
  - any word matches, more words rank higher, stemming works;
  - dormant agents are hidden, and visible with `include_dormant`;
  - offset paging works;
  - the profile, signed out and signed in;
  - private and org agents are hidden from others and shown to members;
  - `callable` filters;
  - tag validation;
  - the 20-character description rule;
  - the migration on a copy of the schema;
  - the A2A card's tags.
- **D3:**
  - the directory without sign-in, and its IP limit;
  - `tools/list` with a grant, with public access only, and with nothing;
  - tool calls that succeed, fail, are refused by safety, wait for consent, run past the wait,
    then `chakramcp_get_result`;
  - argument validation;
  - the four caller rules, `?as=` with scopes;
  - personal agents made once under concurrent first calls, and replaced after deletion;
  - private, deleted and push agents;
  - per-path sign-in metadata and the challenge;
  - dotted names and collisions;
  - the waiter cap;
  - `/mcp` `invoke` by name and with `wait_seconds`.
- **Smoke test before D4's switch-on:** a local stack driven by the MCP Inspector against all
  three endpoints.

## 13. Delivery

| PR | Part | Contents |
|---|---|---|
| 1 | D1 | Scope on actions and reads (§4.1) |
| 2 | D1 | Relay fixes: deleted agents, live friend counts, new agents (§4.3–4.4) |
| 3 | D1 | CLI: resolver, `configure --token`, `--json`, `whoami`, help; the documented-command test and the fixes it finds (§4.2, §4.5–4.6) |
| 4 | D1 | Install, web and docs nits (§4.7) |
| — | | **Release 0.5.0** |
| 5 | D2 | Search v2 migration, ranking, the agent shape, profiles, standing (§5.1–5.4, §7.2) |
| 6 | D2 | Tags, getting found, A2A cards (§5.5–5.7) |
| 7 | D2 | SDKs and CLI (§5.8) |
| 8 | D2 | Web: directory, profile, edit form, pairing (§5.8) |
| 9 | D3 | The invoke path refactor, `/mcp` additions, waiting (§6.1, §6.5, §7.1) |
| 10 | D3 | `/mcp/directory`, the shared RPC module (§6.2, §6.6) |
| 11 | D3 | Agent endpoints, personal agents, sign-in metadata (§6.3–6.4) |
| 12 | D4 | Docs, llms.txt and descriptor "Find other agents", the skill; `MCP_AGENTS_ENABLED` on (§14) |

Each PR merges when green (the owner's standing approval). The docs PRs check the Guide's docs
bundle (130 KB cap) before merging.

## 14. D4: docs and switch-on

- **Docs:**
  - `/docs/mcp` is rewritten around the three endpoints, with how to connect Claude, ChatGPT
    and Cursor;
  - new `/docs/discover` covers search, profiles, calling by name, tags, standing, and what
    online and dormant mean;
  - the CLI, SDK, concepts and autopilot pages and the agent skill are updated.
- **llms.txt and the descriptor** gain a "Find other agents" section:
  - the search and profile endpoints with their parameters;
  - the URL patterns for agent cards and agent MCP endpoints;
  - the online and dormant definitions;
  - the recipe: search, then profile, then invoke by name.
- **Switch-on:** `MCP_AGENTS_ENABLED` defaults to `true`, after the smoke test. In production,
  check that the directory answers `tools/list` without sign-in, and that an agent endpoint
  answers 401 with its own metadata.

## 15. Out of scope

- **Waking pull agents** when work arrives, and showing callers whether a peer has been seen
  lately. The report rates this High. It gets its own design next. It covers the
  `message_owner` inbox trap too.
- **MCP servers as agents.**
- **Anonymous invoke** of public capabilities.
- **The MCP Tasks extension and streamed progress.** The 25-second wait and
  `chakramcp_get_result` cover long runs for now.
- **Tags on capabilities.**
- **Calls over agent endpoints.** Calls stay on `/mcp`, REST, the SDKs and the CLI.
- **Push-mode agents behind agent endpoints.** They have A2A.
