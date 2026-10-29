# Waking pull agents W3: callers see liveness — implementation plan

Derived from `2026-10-28-waking-pull-agents-design.md` (§8, "Liveness for callers"). W3 is
PR 3 of §15: a `peer` object on open invocations, without `wake` (that waits for PR 6's
table).

**Branch:** `feat/w3-peer-liveness`.

## The presence map (`relay/src/heartbeats.rs`)

`Heartbeats` keeps a second map that is never drained: `agent → last_seen`.

- **`record()`** writes both maps under one lock.
- **`reload(db)`** reads `agent_heartbeats` rows seen in the last 30 days and keeps whichever
  time is newer, memory or database. Entries older than 30 days are dropped, so the map holds
  only recently seen agents.
- **The flusher** reloads after each flush, every 30 seconds. A failed reload logs and keeps
  the old map.
- **At startup,** both binaries load it once before serving. A failure logs and the relay
  starts anyway; the next tick fills it.
- **`peer(agent, mode)`** is a lookup with no I/O.
- **`ONLINE_WINDOW`** is 5 minutes, matching the directory's `interval '5 minutes'`.

## `peer`

```rust
#[serde(tag = "delivery", rename_all = "lowercase")]
enum Peer { Pull { online: bool, last_seen_at: Option<DateTime<Utc>> }, Push }
```

This serializes as `{"delivery":"pull","online":false,"last_seen_at":…}` or
`{"delivery":"push"}`.

**Shown when:**
- the invocation is open (`pending`, `in_progress` or `awaiting_consent`);
- the reader is on the calling side (`i_invoked`).

**Where:**
- **REST:**
  - the invoke response (`InvokeResponse.peer`);
  - `GET /v1/invocations/{id}` and the list (`InvocationDto.peer`).
  - The invoke paths already read the target's mode. The reads add `ga.mode` to queries
    they already run.
- **MCP:**
  - `invoke` and `poll_invocation` (through `invocation_view`);
  - `list_invocations`;
  - the agent endpoint's results for open invocations (`chakramcp_get_result`, and tool
    calls still running). `peer` is added to the tools' output schema.
- **A2A:**
  - `GetTask` and the task a parked `message/send` returns, in
    `metadata["chakramcp.peer"]`.
  - `get_task` gains the presence map and the row's target.

## Hints

Hints are plain text, never JSON. They appear only for a pull agent that isn't online:
- "*acct/agent* was last seen 3 hours ago, so it may not pick this up soon."
- "*acct/agent* hasn't been seen in the last 30 days, so it may not pick this up soon."

**MCP:**
- `invoke` and `poll_invocation` add the hint as a second text item after the JSON.
- The agent endpoint appends it to its "Still running" text.
- The handle comes from the queries already run: the invoke paths add the target's account
  slug, and `invocation_view` joins it.

**CLI:**
- `invoke --wait`, `invoke wait` and `invoke ensure --wait` print the hint to stderr once,
  from the first row that has `peer`.
- They use `granter_display_name`.
- The CLI doesn't depend on `shared`, so it has its own small formatter, with its own tests.

## Not in W3

- `peer.wake` and its hint: PR 6.
- SDK types for `peer`: W4, which changes the SDKs anyway (`release`, `expires_in`).
  - TypeScript and Python already pass the field through.
  - Go and Rust drop it until then.

## Tests

- **Heartbeats:**
  - `record` updates presence;
  - `reload` merges newer times both ways and drops entries older than 30 days;
  - a failed flush still leaves presence current.
- **`peer` matches the directory:** agents seen 4 and 6 minutes ago, never seen, and push.
  The directory's `online` and `peer.online` agree for each.
- **REST:**
  - an invoke response has `peer`;
  - `GET` shows it to the caller but not to the target side;
  - it's gone once the invocation settles;
  - the list shows it on open rows only.
- **Reload:** a time written to `agent_heartbeats` by "another copy" shows up after
  `reload`.
- **MCP:**
  - `invoke` and `poll_invocation` carry `peer` and the hint;
  - an online agent gets no hint.
- **Agent endpoint:** a pending result carries `peer`.
- **A2A:** `GetTask` metadata has `chakramcp.peer` while open, and none after.
- **CLI:** the hint formatter's wording at minutes, hours, days, and never seen.
