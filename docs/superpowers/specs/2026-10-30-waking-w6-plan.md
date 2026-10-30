# Waking pull agents W6: wake URLs — implementation plan

Derived from `2026-10-28-waking-pull-agents-design.md` §5 (`d`), §7, §8 (`peer.wake`) and
§12 (0055). W6 is PR 6 of §15, split in two:

- **W6a:** the relay, MCP and CLI.
- **W6b:** the web settings form, and `Peer.wake` in the SDKs.

**Branches:** `feat/w6a-wake-urls`, then `feat/w6b-wake-urls-web`.

## W6a

### Migration 0055 `agent_wake`

- **Columns:** the table from §7.
- **One change: no foreign key to `agents`.**
  - Creating one would lock `agents`, as 0039 (`agent_heartbeats`) explains.
  - A row for a deleted agent is harmless, because the dispatcher and the API join
    through `agents` and skip tombstoned ones.

### The hub's dispatch tap (`shared::wake`)

- **When the tap is on:** `Wake::start_dispatching()` turns it on in a process that runs
  a dispatcher.
- **What `notify` does:** with the tap on, it also adds the agent to a dispatch set.
  That's a set insert, with no I/O.
- **The bus payload gains `d`** (`#[serde(default)]`): true when the sender has no
  dispatcher, which is the standalone app binary.
- **What a receiver does with it:** a process that dispatches takes a remote entry's
  agent only when `d` is true.

### Settings

- **New setting:** `WAKE_URLS_ENABLED` (default `true`).
  - With it off, no dispatcher runs and `PUT …/wake` answers 409.
  - It goes in `RelayState.wake_urls`, with `https_only` (true on a managed server).
- **Constants:**
  - the 2-second grouping window;
  - the 30-second throttle;
  - a 5-second send timeout;
  - a limit of 20 failures.
- **For tests:** the group and throttle windows are settable.

### The dispatcher (`relay/src/wake_urls.rs`)

There's one per relay process. It takes the hub's dispatch set and skips agents with no
enabled URL, using the presence map (below), so most notifies cost nothing. For each
remaining agent:

1. **Group:** the agent is due 2 seconds after its first wake.
2. **Count:** pending invocations, undelivered notices and undelivered call entries, in
   one read. If the count is zero, nothing is sent.
3. **Claim the slot:**
   - `UPDATE agent_wake … last_sent_at = now() WHERE enabled AND (last_sent_at IS NULL OR
     last_sent_at < now() - throttle) RETURNING url, format, secret, handle`. The handle
     is joined from the agent and its account, skipping tombstoned agents.
   - If the slot is taken, the agent is due again when the window ends.
4. **Send:** through `outbound::client_builder(state.outbound)`, with a 5-second
   timeout.
   - **JSON format:** the body from §7, with
     `ChakraMCP-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, "t.body")>`.
   - **Text format:** `"{handle} has N new items on ChakraMCP"`, sent as `text/plain`.
5. **Record the result:**
   - A 2xx resets `failures` and sets `last_ok_at`.
   - Anything else adds one to `failures` and sets `last_error`.
   - At 20 failures the URL is disabled (`disabled_at`), the agent gets a
     `wake_url_disabled` notice, and it leaves the presence map.

**Metric:** `chakramcp_wake_sent_total{result}`, where the result is `ok`, `failed`,
`throttled` or `nothing_pending`.

### REST (`relay/src/handlers/agent_wake.rs`)

**Who can use it:** membership of the agent's account plus the credential's scope, as
for `PATCH /v1/agents/{id}`.

- **`GET /v1/agents/{id}/wake`** returns `{url, format, enabled, failures, last_sent_at,
  last_ok_at, last_error, disabled_at}`, without the secret. With no URL set it's a 404.
- **`PUT /v1/agents/{id}/wake`** takes `{url, format?}`.
  - **Refused with 400:**
    - push agents;
    - a URL the outbound guard refuses (`check_url`), with the guard's message;
    - `http:` on a managed server;
    - any format other than `json` or `text`.
  - **What it does:** an upsert that sets a new secret, turns the URL on and clears
    failures. It answers with the row plus `secret` (`cmw_…`), shown this once.
- **`DELETE /v1/agents/{id}/wake`:** 204.
- **`POST /v1/agents/{id}/wake/test`** pings now, whatever the counts and the throttle.
  It records the result as a real ping does, and answers `{ok, status, error}`.

All of these update the presence map's `wake` set at once. Other relay copies catch up
within 30 seconds.

### `peer.wake` and the hint

- **The presence map:** `Heartbeats` gains a `wake` set, reloaded with presence from
  `agent_wake WHERE enabled` (non-tombstoned).
- **The field:** `Peer::Pull` gains `wake: bool`.
- **The hint, for a pull agent that isn't online:**
  - **No wake URL:** "*acct/agent* was last seen 3 hours ago and has no wake URL, so it
    may not pick this up soon."
  - **A wake URL:** "*acct/agent* was last seen 3 hours ago; it has a wake URL, so
    ChakraMCP pings it."
- **The CLI's formatter** follows.

### MCP and CLI

- **MCP `update_agent`** takes `wake_url` (a string, or null to clear) and `wake_format`.
  The answer carries `wake`, with the secret once when it's set.
- **CLI:**
  - `agents wake show <agent>`;
  - `agents wake set <agent> --url … [--format json|text]`;
  - `agents wake test <agent>`;
  - `agents wake clear <agent>`.

### Tests (against a local test server)

- **The ping:**
  - the signature verifies with the secret from `PUT`;
  - the text format;
  - zero counts send nothing.
- **Throttling:**
  - the throttle holds across two relay states;
  - a throttled wake is checked again and pinged after the window.
- **Failures and refusals:**
  - 20 failures disable the URL, with its notice;
  - a push agent is refused, as are a private address under a public-only policy and
    `http:` on a managed server;
  - only members read the URL, and the secret is shown only on `PUT`.
- **End to end:** an invoke pings a waiting test server.
- **`d`:**
  - a remote wake with `d` reaches the dispatcher;
  - one without `d` doesn't.
- **`peer.wake`** and its hints; MCP `update_agent`; CLI parsing.

## W6b

- **The agent settings page:**
  - the wake URL form (URL, format, test, clear);
  - its status (last OK, last error, turned off);
  - the secret shown once.
- **SDKs:** `Peer.wake` (optional).
