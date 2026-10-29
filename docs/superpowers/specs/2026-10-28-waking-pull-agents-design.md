# Waking pull agents

**Status:** design approved section by section on 2026-10-28, then revised after the spec
review (§17).

**Background:**
- This design answers the Grok-bot agent-to-agent test report (2026-10-26), which rated
  slow pull delivery **High**.
- It also fixes the report's "message_owner inbox trap".
- The outbound URL guard it relies on shipped separately in #473.

## 1. Why

### Pull agents only learn about work by polling

- A pull agent finds invocations, notices and call events only when it calls one of
  `/v1/inbox`, `/v1/inbox/notices` or `/v1/inbox/calls`.
- Nothing on the relay can hold a request open or ping an agent.
- The SDK serve loops poll every 2 seconds, and the house agents every 5. Each poll writes
  a usage row.
- In the report, one chat assistant invoked another:
  - the call sat pending for 4 minutes until a person ran `inbox pull` by hand;
  - once both sides were polling, a round trip took 1.7 seconds.

### Callers can't see whether a peer is around

- Pulls record a heartbeat in memory, and every 30 seconds it is written to
  `agent_heartbeats`.
- The directory shows `online` and `last_seen_at`.
- An invoke response or a pending invocation tells the caller nothing about the peer.

### The `message_owner` trap

The report puts it as "inbox pull permanently claims message_owner calls".

- **Looking claims the work.** Looking at the inbox through `inbox pull` claims the row. The row moves to
  `in_progress` and is never handed out again.
- **Nothing times out.** Pending and claimed rows wait forever, and so does the caller.
- **The owner's web reply is hard to reach:**
  - The web Inbox can answer a call, but only once it's `in_progress`.
  - Its form takes raw JSON.
  - Nothing points the owner to it.
  - Self-hosted servers have no inbox page at all.

### The relay can run several copies

The chart allows replicas. A wake held in one process's memory doesn't reach a pull
waiting on another copy.

### The rule this design keeps

The invocation path only reads in-memory state. Metering, accounting, and every new
database or network step run in background tasks.

## 2. Decisions (the owner's)

### Who gets woken (Q1 = B)

- **Agents with a running process** use waiting pulls (long-poll).
- **Agents without one** (chat assistants, scripts that aren't running) can set a **wake URL**:
  - the relay pings it when work arrives;
  - the ping holds counts only, never content.
- **Out of scope:** email and browser push to the owner. An ntfy-style wake URL already
  reaches a phone.

### The trap (Q2 = A)

- The owner can answer `human_in_loop` calls on the web, even ones nobody has claimed.
- Agents can look without claiming, and hand a claim back.
- Leases return claims that go stale.

### Deadlines (Q3 = A)

- Every invocation gets a deadline: 24 hours by default, 7 days for `human_in_loop`.
- The caller can shorten it with `expires_in`.
- At the deadline, open work becomes `timeout`, with a reason.

### Mechanism (approach 1)

- A `wait` option on the existing pulls.
- Rejected: a new wait endpoint, and a server-sent event stream.
- Why this one: it suits every client, including a CLI in cron and MCP chat clients, and
  it reuses today's claim logic.

### Credits

- A timed-out call stays charged, like a failed one.
- Refunds would need a refund kind in the credit ledger, so they wait until callers ask
  for them.

## 3. Overview

**The seven pieces:**

1. **The wake hub** (`shared/src/wake.rs`, §4): an in-memory signal per agent and per
   feed. It lives in `shared` because notify sites run in code that the app crate also
   calls.
2. **The cross-process task** (§5): carries wakes between processes through Postgres
   `LISTEN/NOTIFY`.
3. **Waiting pulls** (§6): `wait` on the three inbox feeds and the matching MCP tools.
4. **Wake URLs** (§7): a per-agent URL, pinged when work arrives and is still unclaimed.
5. **Liveness for callers** (§8): a `peer` object on unsettled invocations, read from
   memory.
6. **Claims** (§9): looking without claiming, release, leases, and the owner's reply on
   the web.
7. **Deadlines** (§10): `expires_at` on invocations, ended by a background sweep.

**What the invoke path gains:**
- one `wake.notify` call after its existing commit;
- a memory read for `peer`.

It does no new database work and makes no network calls.

## 4. The wake hub

### Where it lives

- The hub is `chakramcp_shared::wake::Wake`, a cheap clone handle.
- `chakramcp-server` runs app and relay in one process and builds **one** `Wake`, shared
  by `AppState` and `RelayState`.
- The standalone `chakramcp-app` and `chakramcp-relay` binaries (dev only) each build
  their own.

### Feeds

`Feed` is one of:
- `Invocations`, for `/v1/inbox`;
- `Notices`, for `/v1/inbox/notices`;
- `Calls`, for `/v1/inbox/calls`.

### `wake.notify(agent_id, feed)`

It's a plain function, not `async`, so it can't do I/O. It:
1. bumps a generation counter for `(agent, feed)` and signals any pull waiting on it;
2. inserts `(agent, feed)` into the outbound set that the cross-process task drains (§5).

The set is keyed by `(agent, feed)`, so a burst collapses into one entry, and it can't
grow past the number of active agents.

### Waiting

- A pull subscribes before its first claim, recording the generation, then waits for the
  generation to change. Work that arrives between the claim and the wait is still caught.
- A key's entry is removed when its last waiter leaves.
- A waiting pull holds no database connection while it waits. It holds one only for each
  claim.

### Waiter cap

A process-wide count of waiting pulls, capped by `INBOX_MAX_WAITERS`.

### Shutdown

- `wake.close()` releases every waiter. Each returns `[]` with `ChakraMCP-Wait: 0`
  without claiming again, and new waits return at once.
- It's called from `chakramcp-server`'s SIGTERM handler. The standalone relay binary gets
  the same handler.
- A deploy isn't held up 25 seconds.

### Where `notify` is called

Always after the commit that queues the work. Each PR adds the sites for the work it
introduces.

| Event | Agent and feed | PR |
| --- | --- | --- |
| Invoke (REST, MCP, the agent endpoint) inserts a `pending` row | called agent, `Invocations` | 1 |
| An A2A request is parked for a pull agent | called agent, `Invocations` | 1 |
| An approval moves a held request to `pending` (`shared::consent::decide::approve`, used by the relay and the app's built-in page) | called agent, `Invocations` | 1 |
| Every existing `INSERT INTO agent_notices` (consent enqueue, consent expiry, reply notices) | that agent, `Notices` | 1 |
| A call entry delivered to a party (invite, message, accept, decline, end, missed) | recipient, `Calls` | 1 |
| Release, or a lease expiring back to `pending` (§9) | called agent, `Invocations` | 4 |
| The sweep's `invocation_expired` notice, and the `answered_by_owner` notice | called agent, `Notices` | 4, 5 |
| The `wake_url_disabled` notice | that agent, `Notices` | 6 |

Workers that change many rows use `RETURNING` to get the agent ids, and call `notify`
for each.

## 5. The cross-process task

There's one per process (`shared/src/wake.rs`). It keeps a dedicated `PgListener`
connection outside the pool.

### Sending

- It drains the outbound set about every 50 ms.
- It sends `pg_notify('chakramcp_wake', payload)` in batches. Each payload stays under
  7 KB (about 160 entries), below Postgres's 8 KB limit.
- The payload is `{"o":"<process id>","d":<bool>,"w":[["<agent id>","i"|"n"|"c"], …]}`:
  - `o` is the sending process;
  - `d` is true when the sender runs no wake-URL dispatcher (the standalone app binary).

### Receiving

- It ignores its own messages (matching `o`).
- For every entry, it signals local waiters as `notify` does, but doesn't send the entry
  on again.
- It hands an entry to the local wake-URL dispatcher (§7) only when `d` is true. Wakes
  that start in a process with a dispatcher are dispatched there.

### Failures

- **A failed `pg_notify`** is logged, counted and retried with the next batch.
- **A dropped listener** reconnects with backoff, then wakes every local waiter so each
  claims again. The wakes are spread over about a second, so up to 2,000 waiters don't
  hit the pool (20 connections) at once.
- **The worst case:** waiters always return at their deadline, so it is today's polling
  delay, never a lost invocation.

### PgBouncer

- `LISTEN` needs a session connection. Behind a PgBouncer in transaction-pooling mode,
  only wakes within one process work.
- The self-hosting docs will say so.

## 6. Waiting pulls

### The API

- `GET /v1/inbox?agent_id=…&wait=25` holds the request open until there's work. The same
  works on `/v1/inbox/notices` and `/v1/inbox/calls`.
- `wait` is in seconds, from 0 to `INBOX_MAX_WAIT_SECS` (25):
  - larger values are clamped;
  - the default is 0, which is today's behaviour.

**How a wait runs:**
1. Subscribe to the hub.
2. Claim as usual. If anything was claimed, return at once.
3. Otherwise wait for a wake or the deadline.
4. On a wake, claim again. If another puller got there first (`SKIP LOCKED`), keep waiting
   until the deadline.
5. At the deadline, return `[]`.

**Details:**
- **One claim function.** The claim SQL exists twice today, in `handlers/invoke.rs` and in
  MCP `pull_inbox`. PR 1 merges them into one function that REST and MCP share. Later PRs
  then change one copy (leases, §9).
- **Telling relays apart.** Every answer carries `ChakraMCP-Wait: <seconds honoured>`. An
  older relay ignores `wait` and sends no header, so a client knows to fall back to
  sleeping between polls.
- **When every slot is taken.** Past `INBOX_MAX_WAITERS`, the relay answers at once with
  `ChakraMCP-Wait: 0`.
- **Heartbeats.** A waiting pull records one when it starts and one when it returns, so a
  waiting agent stays `online`.
- **Usage.** One usage row per request, as now. A waiting agent makes 2–3 requests a minute
  instead of 12–30.
- **Proxies.** 25 seconds is under common proxy read timeouts: 60 seconds for nginx
  ingress, 100 for Cloudflare. Production's Caddy has no read timeout on proxied
  responses. The self-hosting docs will say to keep any proxy timeout above 25 seconds.

### Clients

- **MCP:** `pull_inbox`, `pull_notices` and `pull_calls` take `wait_seconds`, so a chat
  client can wait inside one tool call. `pull_inbox` also takes `lease_secs` (§9).
- **CLI:**
  - `inbox pull --wait 25 [--lease 30m]`, `inbox notices --wait 25` and
    `call pull --wait 25`;
  - a shell loop becomes `while true; do chakramcp inbox pull --wait 25 …; done`.
- **SDKs (TypeScript, Python, Go, Rust):**
  - **`pull` takes `wait` and `leaseSecs`** (in each language's own spelling) and reports
    whether the relay waited. That's for agents with their own loop, like the house
    agents.
  - **`inbox.serve` and `calls.serve`** send `wait=25`:
    - while the header says the relay waits, they loop straight away;
    - without the header, they sleep for the poll interval as now, so they still work
      against 0.6.0 and older servers;
    - `serve` takes `leaseSecs` for handlers that run longer than the default lease.
  - **Errors** back off as today.
  - **Notices** keep their 15-second interval inside `serve`.
- **House agents:**
  - They run their own loop over the published `@chakramcp/sdk` (0.6.0) `pull`.
  - After the release that ships PR 2, they pass `wait`.
  - That makes them instant and removes about 35,000 idle poll rows a day.

### Lost responses

- If a client disconnects between a claim and the reply, the claimed work is lost to it,
  as with any pull today.
- Leases (§9) return calls that aren't `human_in_loop` after 10 minutes.
- `human_in_loop` calls stay answerable on the web.

## 7. Wake URLs

### Storage

A new table, `agent_wake` (migration 0055), so the hot `agents` table isn't altered:

```sql
CREATE TABLE agent_wake (
    agent_id     UUID PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    url          TEXT NOT NULL,
    format       TEXT NOT NULL DEFAULT 'json' CHECK (format IN ('json', 'text')),
    secret       TEXT NOT NULL,          -- HMAC key; shown once when set
    enabled      BOOLEAN NOT NULL DEFAULT true,
    failures     INTEGER NOT NULL DEFAULT 0,
    last_sent_at TIMESTAMPTZ,
    last_ok_at   TIMESTAMPTZ,
    last_error   TEXT,
    disabled_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

The secret is stored the way the relay's signing keys are today: in the database,
without encryption at rest.

### Setting one

- **Who can set it:** anyone who can manage the agent.
- **Which agents:** pull agents only. Push agents already get their calls forwarded, so
  setting a URL on one gets a 400.
- **REST:**
  - `PUT /v1/agents/{id}/wake`, with body `{url, format}`;
  - `DELETE /v1/agents/{id}/wake`;
  - `POST /v1/agents/{id}/wake/test`, which sends a ping now and reports the result.
- **MCP:** `update_agent` with `wake_url` and `wake_format`. Null clears it.
- **CLI:** `agents wake set|test|clear`.
- **Web:** the agent's settings page.
- **SDKs:** no change. Agents built on an SDK run a process and use waiting pulls.

**Checks on the URL:**
- It must pass the outbound guard from #473 (`outbound::check_url`).
- On a `managed` server, it must be https.
- A refused URL gets the guard's 400.

**Who sees it:**
- The URL may contain a bot token, so it's treated as a secret. Only members of the
  agent's account can read it.
- The secret is returned once, when the URL is set. Setting the URL again makes a new
  secret.

### The ping

**JSON (the default):**
- The body is
  `{"type":"chakramcp.wake","agent":"acct/slug","agent_id":"…","pending":{"invocations":2,"notices":0,"calls":1},"sent_at":"…"}`.
- The header is `ChakraMCP-Signature: t=<unix>,v1=<hex HMAC-SHA256 of "t.body">`.

**Text (`format: text`):**
- A plain line, for example "bob/venue-bot has 3 new items on ChakraMCP".
- That turns an ntfy topic into a phone notification.
- A Telegram bot works too, with `chat_id` and `text` in the URL.

**Nothing private:** no content and no caller identity, only counts and the agent's own
name.

### When it's sent

There's one dispatcher per relay process. It's off when `WAKE_URLS_ENABLED=false`. Its
inputs are local wakes, plus remote wakes marked `d` (§5).

1. **Group.** Wait about 2 seconds after the first wake for an agent, which groups a burst
   into one ping.
2. **Count.** Count the agent's unclaimed work in a background read: pending invocations,
   undelivered notices, undelivered call entries. If the count is zero, send nothing.
3. **Claim the slot.** This gives at most one ping per agent every 30 seconds, from
   exactly one relay copy:

   ```sql
   UPDATE agent_wake SET last_sent_at = now()
    WHERE agent_id = $1 AND enabled
      AND (last_sent_at IS NULL OR last_sent_at < now() - interval '30 seconds')
   ```

4. **If the slot was taken** (throttled), check the agent again when the 30-second window
   ends. Work that arrived inside the window still gets a ping, with fresh counts, if it's
   still unclaimed.
5. **Send** through `outbound::client_builder`, with a 5-second timeout.

### Failures

- A non-2xx answer or a network error records `last_error` and adds one to `failures`.
- A success resets `failures` and sets `last_ok_at`.
- After 20 failures in a row:
  - `enabled` becomes false and `disabled_at` is set;
  - the agent gets a `wake_url_disabled` notice;
  - the web page shows it.
- Setting the URL again turns it back on.

### Out of scope, and the metric

- **Out of scope:** reminder pings for work that stays unclaimed. A ping goes out when work
  arrives, and deadlines (§10) end anything left waiting.
- **Metric:** `chakramcp_wake_sent_total{result}`.

## 8. Liveness for callers

### What callers see

A `peer` object appears on every invocation that isn't settled yet, when the target is a
pull agent:

```json
"peer": { "delivery": "pull", "online": false, "last_seen_at": "2026-10-28T09:12:03Z", "wake": true }
```

- **`online`** uses the directory's definition: last seen within 5 minutes. A test keeps the
  directory's SQL interval and the in-memory constant in step.
- **`last_seen_at`** is the agent's last pull, waiting pulls included. It's null if the agent
  hasn't been seen in 30 days.
- **`wake`** says whether the agent has an enabled wake URL. It's added in PR 6, with the
  table it reads. Until then it's absent.
- **Push targets** get `{ "delivery": "push" }`.
- **When it's left out:** once the invocation settles, and whenever the reader isn't the
  caller.

### Where it appears

- **REST:** the invoke response, `GET /v1/invocations/{id}`, and the list.
- **MCP:** `invoke`, `poll_invocation`, and the agent endpoint's `chakramcp_get_result`.
- **A2A:** `GetTask`, in the task's `metadata["chakramcp.peer"]`.
- **Plain-language hints** in MCP text results and the CLI, for example "bob/venue-bot was
  last seen 3 hours ago, so it may not pick this up soon".
  - From PR 6 the hint also mentions a missing wake URL.
  - It goes in text only, never in the JSON.
  - The CLI's `invoke --wait` shows it while it waits.

### Keeping the invoke path in memory

`Heartbeats` gains a presence map that is never drained: `agent → { last_seen, wake }`.

- **On a pull:** `record()` updates it under the same lock as the heartbeat.
- **Every 30 seconds,** after its flush, the flusher reloads it:
  - `last_seen` comes from `agent_heartbeats` (agents seen in the last 30 days), keeping
    whichever is newer, memory or database;
  - from PR 6, `wake` comes from `agent_wake WHERE enabled`.
- **At startup,** it's loaded once before the relay serves.
- **Across copies,** every relay copy agrees within about 30 seconds.
- **Readers:** the invoke path and invocation reads take `peer` from this map with a lookup
  and no I/O, so their answers match.

**What's exposed:**
- `online` and `last_seen_at` are already public in the directory for network agents.
- A caller sees `peer` only for agents it can call.
- `wake` reveals only that a URL exists.

## 9. Claims

### Looking without claiming

This already exists, and it's safe on every server version:
- REST `GET /v1/invocations?direction=inbound&status=pending&agent_id=…`;
- MCP `list_invocations`;
- CLI `invocations list`.

So no `claim=false` is added. On an older relay, an unknown query field would be ignored
and the rows claimed, which is the trap itself.

What's new:
- The CLI gets `inbox list --agent …`, an alias for
  `invocations list --direction inbound --status pending`.
- The skill and docs (PR 7) tell chat assistants to list first, with `list_invocations`,
  and to claim only what they'll answer now.

### Release: handing a claim back

`POST /v1/invocations/{id}/release` puts a claimed call back to `pending`. It:
- clears `claimed_at`, `claimed_by_user_id` and `lease_until`;
- sets `deliveries = GREATEST(deliveries - 1, 0)`, so a voluntary hand-back doesn't count
  toward the cap;
- wakes the agent's `Invocations` feed.

**Who and when:**
- Anyone who can act as the called agent may release: account membership plus the
  credential's scope, as for results.
- The row must be `in_progress`, with no pending reply draft. Otherwise the answer is
  `409 chk.invocation.not_releasable`.

**Clients:** MCP gets `release_invocation`, and the CLI gets `inbox release <id>`.

### Leases: claims that go stale

**New columns** on `relay_invocations` (§12):
- `lease_until TIMESTAMPTZ`;
- `deliveries INTEGER NOT NULL DEFAULT 0`.

**Claiming** (the shared claim function from §6):
- adds one to `deliveries`;
- sets `lease_until = now() + lease`, where the lease is `CLAIM_LEASE_SECS` (600) by
  default, or `lease_secs` on the pull, clamped to 24 hours;
- leaves `lease_until = NULL` for `human_in_loop` calls, which never expire this way.

**The sweep** (§10) runs leases in the same task, every 30 seconds, in batches with
`SKIP LOCKED`:
- An `in_progress` row past `lease_until` goes back to `pending`, with its claim fields
  cleared, and the agent is woken.
- If `deliveries >= 3`, it becomes `timeout` instead, with
  `error_message = 'claimed 3 times, never answered'`.

**Accepted behaviour change.** Leases apply to every claim, including those by 0.6.0
clients that can't pass `lease_secs`:
- A handler that runs longer than 10 minutes can see its call handed out again, and after
  three rounds it times out.
- Operators can raise `CLAIM_LEASE_SECS`.
- New clients pass `lease_secs`.
- The release notes will say this.

**Late answers.** `submit_result` accepts a result while the row is:
- `in_progress`; or
- `pending` with `deliveries > 0`: it was claimed before and its lease ran out.

Work finished after its lease still lands, and the first answer wins.

`submit_result`'s final `UPDATE` today ignores `rows_affected`. It must check it, so that
losing a race with the sweep or another answer returns
`409 chk.invocation.already_settled`, not a success.

### The owner answers on the web (`human_in_loop`)

**What exists today:**
- An approver signed in to the web can already answer a call. `submit_result` sends them
  straight to `reply::confirm_now`, which stores the reply as confirmed by them and
  releases it.
- The web Inbox shows that form, but only for `in_progress` rows.

**What's added:**
- **Pending rows.** `submit_result` lets an interactive approver's reply to a
  `human_in_loop` row through when the row is `pending`, as well as `in_progress`. Every
  other caller keeps the rules above.
  - `confirm_now` then first claims the row, in its own transaction: `in_progress`,
    `claimed_at = now()`, `claimed_by_user_id` = the owner, and `deliveries + 1`.
  - Then it releases the reply as today.
  - `reply::release` keeps its `status = 'in_progress'` guard.
- **The web Inbox** shows its reply form on inbound `pending` `human_in_loop` rows too, with
  a plain text box next to the JSON one. Other capabilities still need a claim first.
- **The approvals page.** Its "Waiting" list gains *messages*. These are `human_in_loop`
  calls, pending or claimed and still unanswered, to agents whose capability the user may
  decide for, under the same `consent_approvers` rule the list already uses.
  - Each shows the caller, the capability, the input and a reply box.
  - The box posts to the existing result endpoint.
  - An invocation that already has a reply draft waiting shows as that draft, not twice.
- **Self-hosted servers.** The app's built-in approvals page (`app/src/pages/approvals.rs`)
  gets the same messages list and reply box.
- **What readers see:** `human_confirmed`, with its token, as for any confirmed reply. No
  new field.
- **The agent is told.** When an owner's reply settles a row that was `in_progress`, the
  agent gets an `answered_by_owner` notice, and a result it posts later gets the 409.
- **Non-`human_in_loop` calls** are left as they are. They don't appear on the approvals
  page.

### Left as they are

The notices and calls feeds still hand each item out once:
- notices are informational;
- a call's messages can be re-read with `call show`.

So neither can trap work the way the inbox did.

## 10. Deadlines

### Setting one

`relay_invocations` gains `expires_at TIMESTAMPTZ` (§12).

**At invoke.** The invoke path's existing `INSERT` sets `expires_at = now() + ttl`,
computed in memory:
- **The caller's `expires_in`,** if given:
  - longer than the default is clamped to it;
  - under 10 seconds is a 400.
- **Otherwise the default:**
  - `INVOCATION_DEFAULT_TTL` (24 hours);
  - `INVOCATION_HITL_TTL` (7 days) for `human_in_loop` calls.

**Held calls (`awaiting_consent`).**
- The clock starts when approval moves the row to `pending`.
- `decide::approve`'s release update sets
  `expires_at = now() + (expires_at - created_at)`, which keeps the requested length
  without another column.
- Consent requests keep their own wait, as now.

**How callers set it:**
- REST `expires_in` (seconds), and MCP `invoke` `expires_in`;
- A2A `metadata["chakramcp.expires_in"]`;
- CLI `--expires-in 2h`;
- SDKs `invoke(…, expiresIn)`, in each language's own spelling.

### Notices about invocations

- Today a notice exposes only `{id, kind, created_at, consent_request}`.
- It gains `invocation_id`, read from the payload's `invocation_id` and null when absent.
- Notices about an invocation carry it: `invocation_expired` and `answered_by_owner`.
  `wake_url_disabled` has none.
- The four SDKs' `Notice` types gain the optional field.

### The sweep

`relay/src/invocation_sweep.rs`:
- runs every 30 seconds in every relay process;
- works in batches of 500 with `FOR UPDATE SKIP LOCKED`, like the consent expiry worker;
- does deadlines first, then leases.

**Lock order.**
- The deadline pass locks a row's pending reply draft (`consent_requests`) before the
  invocation, the same order the reply-approval path uses. Both use `SKIP LOCKED`.
- A row whose draft is busy waits for the next pass.

**The deadline pass.** `pending` or `in_progress` rows past `expires_at` become `timeout`:
- `error_message`:
  - "picked up, never answered" when `claimed_at` is set;
  - "not picked up before its deadline" otherwise, which includes released rows and
    lease-expired ones that were pended again.
- `elapsed_ms` is set as the consent expiry sets it.
- A pending reply draft for the row is set to `cancelled`.
- The called agent gets an `invocation_expired` notice, so a late worker can stop.
- A late result gets `409 chk.invocation.already_settled`.

**What callers see:**
- `timeout` and its reason, in invocation reads and in MCP. MCP's `wait_for` already
  treats `timeout` as final.
- In A2A, the inbox bridge already maps `timeout` to the task state `failed`.
- Inside a call, the call's `invocation` entry shows `timeout`. Its status is read by
  join, as for every outcome.

**Backfill.**
- Rows from before the migration have no `expires_at`.
- The first passes give each open one a fresh deadline of `now() + ttl` (24 hours, or 7
  days for `human_in_loop`), a thousand at a time, until none are left.
- So rows already trapped, such as unanswered `message_owner` calls, get a full window to
  be answered on the web. They don't all time out in the first pass.

**Metric:** `chakramcp_invocation_expired_total{reason}`, where the reason is
`not_picked_up`, `not_answered` or `lease_exhausted`.

## 11. Errors

| Case | Answer |
| --- | --- |
| A result or release on a settled invocation, or a result that loses a race with the sweep | `409 chk.invocation.already_settled` |
| Release of a row that isn't claimed, or has a pending draft | `409 chk.invocation.not_releasable` |
| `wait` or `lease_secs` above its cap | Clamped. `ChakraMCP-Wait` and the row's `lease_until` show what was used |
| `expires_in` longer than the default | Clamped to the default |
| `expires_in` under 10 seconds, or not a number | 400 |
| Waiter cap reached | An immediate answer with `ChakraMCP-Wait: 0` (not an error) |
| A wake URL the guard refuses, or http on a managed server | 400, with the guard's message |
| A wake URL on a push agent | 400 |

- Both 409s use `ApiError::coded`. Other conflicts keep today's generic `conflict` code.
- **The hub can't hurt invocations:**
  - `notify` does no I/O and can't fail.
  - The outbound set is bounded by the number of active agents.
  - Every `pg_notify`, `LISTEN` or ping failure stays inside a background task, where it is
    logged and counted.

## 12. Data model and migrations

All three migrations are additive.

### 0053 `invocation_lease_columns` (PR 4)

- **Columns** on `relay_invocations`:
  - `ADD COLUMN lease_until TIMESTAMPTZ`;
  - `ADD COLUMN deliveries INTEGER NOT NULL DEFAULT 0`;
  - `ADD COLUMN expires_at TIMESTAMPTZ`.
- **Cost:** a constant default only changes the catalog, so the table isn't rewritten.
- **Lock:** `ACCESS EXCLUSIVE`, taken with 0046's retry loop (100 ms `lock_timeout`, up to
  50 attempts). Nothing else is done under that lock.

### 0054 `invocation_lease_indexes` (PR 4)

- **Indexes:**
  - `relay_invocations_open_expiry ON (expires_at) WHERE status IN ('pending','in_progress')`;
  - `relay_invocations_live_lease ON (lease_until) WHERE status = 'in_progress' AND lease_until IS NOT NULL`.
- **Lock:** `SHARE`, taken with 0047's retry loop. The indexes are built normally, not
  `CONCURRENTLY`, for the reasons 0047 gives. They're in their own migration, as 0046 and
  0047 are split, so the exclusive lock isn't held through the builds.

### 0055 `agent_wake` (PR 6)

The new table from §7. It's a new table, so no lock risk.

### Rollback and the sqlx cache

- The relay refuses to boot on an applied migration it doesn't know. So a rollback
  reverts the code and keeps the migration files. Redeploying an old image doesn't work.
- The sqlx offline cache (`backend/.sqlx`) is regenerated in each PR that adds queries.

## 13. Settings and metrics

| Setting | Default | Meaning |
| --- | --- | --- |
| `INBOX_MAX_WAIT_SECS` | 25 | Longest wait honoured on a pull |
| `INBOX_MAX_WAITERS` | 2000 | Concurrent waiting pulls per process |
| `WAKE_URLS_ENABLED` | true | Run the wake-URL dispatcher |
| `INVOCATION_DEFAULT_TTL` | 24h | Default deadline |
| `INVOCATION_HITL_TTL` | 7d | Default deadline for `human_in_loop` |
| `CLAIM_LEASE_SECS` | 600 | Default claim lease |

Each setting gets a row in `docs/INSTALL.md` and `.env.example`, in the PR that adds it.

**Metrics,** on a new Grafana panel:
- `chakramcp_inbox_waiters` (gauge);
- `chakramcp_wake_notify_total`;
- `chakramcp_wake_listener_reconnects_total`;
- `chakramcp_wake_sent_total{result}`;
- `chakramcp_invocation_expired_total{reason}`.

## 14. Testing

### Hub (unit tests)

- A notify wakes a waiter.
- Work that arrives between the claim and the wait is still caught.
- The waiter cap holds.
- `close()` releases every waiter without a claim.
- The outbound set collapses a burst.

### Waiting pulls (database tests)

- A waiting pull returns within about 100 ms of an invoke, on all three feeds.
- With two pullers and one item, one claims it and the other keeps waiting until its
  deadline.
- The header is present and correct.
- `wait=0` behaves exactly as today.
- An approval made through `shared::consent::decide::approve` wakes the relay's waiter.

### Cross-process

- Two `Wake`s on one database: a notify on one wakes a waiter on the other.
- A listener reconnect wakes every waiter, spread out.
- A process ignores its own messages.

### Claims

- `inbox list` leaves rows `pending`.
- Release works, and returns both 409s where it should.
- When a lease expires, the row goes back to `pending` and the agent is woken.
- The third delivery ends in `timeout`.
- A late result on a re-pended row is accepted.
- A result that loses the race to the sweep gets the 409.
- `human_in_loop` rows get no lease.

### Deadlines

- Both reasons.
- A held call starts its clock at approval.
- A pending draft is cancelled.
- The notice is sent with its `invocation_id`.
- A late result gets the 409.
- The backfill gives fresh deadlines.

### Owner reply

- The owner can answer a `pending` row and a claimed row.
- The approver role is required, and a non-interactive credential makes a draft (as
  today).
- Safety screening blocks a bad reply.
- The messages list and drafts aren't shown twice.
- The `answered_by_owner` notice is sent.
- The agent's later result gets the 409.

### Wake URLs (against a local test server)

- The signature verifies.
- The text format works.
- Zero counts send nothing.
- The 30-second throttle holds across two relay states, and a throttled wake is checked
  again.
- The URL is switched off after 20 failures, with its notice.
- The guard refuses private URLs on a managed server.

### `peer`

- It matches the directory's `online`.
- It reloads from the database.
- It's left out once the invocation settles, and for readers who aren't the caller.

### Clients

- Each SDK's `serve` waits when the header is present and sleeps without it.
- `pull` passes `wait` and `leaseSecs`.
- The CLI's documented-commands test covers every new command.
- The frontend tests cover the pending-row reply form, the approvals messages, and the
  wake URL form.

### Load (by hand, before rollout)

2,000 waiting pulls on a local relay, recording memory, CPU and pool waits.

## 15. PRs and rollout

Each PR merges when CI is green.

1. **Hub and waiting pulls:**
   - `shared/src/wake.rs` and the cross-process task;
   - one hub shared by `AppState` and `RelayState`;
   - `notify` at the PR 1 sites in §4;
   - the merged claim function;
   - `wait` on the three feeds and the MCP tools, and CLI `--wait`;
   - heartbeats on waits, the header, the cap, shutdown;
   - setting rows, metrics.
2. **SDKs** in TypeScript, Python, Go and Rust:
   - `pull` gains `wait` and `leaseSecs`;
   - `serve` waits;
   - `Notice` gains `invocation_id`.
3. **`peer` liveness**, without `wake`:
   - the presence map;
   - the REST, MCP and A2A fields;
   - the CLI and MCP hints.
4. **Claims and deadlines:**
   - migrations 0053 and 0054;
   - release, leases, the sweep, `expires_in` everywhere;
   - late-result handling with the `rows_affected` check;
   - `invocation_id` on notices;
   - `inbox list`.
5. **The owner's reply on the web:**
   - `confirm_now` on pending rows;
   - the Inbox form on pending rows;
   - approvals messages, on the web and on the self-hosted page;
   - the notice.
6. **Wake URLs:**
   - migration 0055 and the dispatcher;
   - REST, MCP, CLI and the web settings form;
   - `peer.wake` and its hint.
7. **The skill and docs:**
   - the poll loop, listing before claiming, wake URLs;
   - proxy timeouts and PgBouncer;
   - the `step-4-automation` page and the agent skill.

   The Guide bundle stays under its 150 KB cap.

**Rollout:**
- Everything is on by default. `wait=0` keeps current clients unchanged, apart from the
  lease change in §9.
- After each deploy, check that commit's CD run, then the new metrics.
- House agents pass `wait` after the release that ships PR 2.
- 0.7.0 is tagged only when the owner asks for a release.

## 16. Out of scope

- Email or browser push to owners.
- Reminder pings for work that stays unclaimed.
- Refunds for timed-out calls.
- Waking callers when a result lands. MCP `wait_for` and invocation reads keep polling.
- Lease renewal. A long job claims with a longer `lease_secs`.
- Re-delivering notices or call entries.
- Encrypting wake secrets at rest. That's the same open item as the signing keys.

## 17. Changes from the sections as presented

### From writing the spec

- **A timeout inside a call adds no new entry** (Section 6 said it would):
  - the call's `invocation` entry already shows `timeout`, because its status is read by
    join;
  - a new entry kind would mean changing the `call_entries` constraint for nothing.
- **Late results** (Section 5 accepted them while the row was `pending` or
  `in_progress`):
  - the `pending` case is narrowed to rows with `deliveries > 0`;
  - so a result can't land on work no one ever claimed.
- **Section 3's throttle** would have dropped a ping for work arriving inside the
  30-second window. Such work now gets a check when the window ends.

### From the spec review, checked against the code

**The owner's reply reuses the existing web reply path.**
- Section 5 proposed a new `owner-reply` endpoint.
- But `submit_result` already lets a signed-in approver answer, through
  `reply::confirm_now`, and the web Inbox offers it for claimed rows.
- So the design extends that path to `pending` rows and adds the approvals messages.
- `answered_by` and the `consent_requests` column are dropped. `human_confirmed` already
  marks an owner's reply.

**Looking without claiming uses the existing invocations list.**
- Section 5 proposed `claim=false`.
- An older relay ignores unknown query fields and would claim the rows.
- The list is safe on every version.

**The hub lives in `shared`.**
- `decide::approve` and the reply notices run in shared code, which the app crate (with
  no relay dependency) also calls.
- `chakramcp-server` shares one hub between its app and relay.

**`peer.wake` moves to PR 6,** with the table it reads.

**The migrations are split.** Columns go under the exclusive lock, and indexes in their own
migration under `SHARE`, as 0046 and 0047 are. `consent_requests` is no longer altered.

**Notices gain `invocation_id`,** so `invocation_expired` and `answered_by_owner` say which
call they're about.

**Smaller fixes:**
- **Backfilled deadlines** are `now() + ttl`, not `created_at + ttl`, so trapped rows
  aren't all timed out on the first pass.
- **The expiry reason** uses `claimed_at`.
- **Release** clears the claim fields and uses `GREATEST(deliveries - 1, 0)`.
- **The sweep** locks drafts in the approval path's order.
- **`submit_result`** checks `rows_affected`.
- **SDK `pull`** takes `wait`, for the house agents' own loop.
- **Shutdown** returns `[]` without claiming.
- **Reconnect** wakes are spread out.
- **Behaviour change:** leases are stated as one for 0.6.0 clients.
