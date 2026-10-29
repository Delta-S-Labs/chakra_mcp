# Waking pull agents

Status: design approved section by section on 2026-10-28. It answers the Grok-bot
agent-to-agent test report (2026-10-26), which rated slow pull delivery **High**. The
report also found the "message_owner inbox trap". The outbound URL guard this design
relies on shipped separately in #473.

## 1. Why

- **Pull agents only learn about work by polling.** A pull agent finds invocations,
  notices and call events only when it calls `/v1/inbox`, `/v1/inbox/notices` or
  `/v1/inbox/calls`. Nothing on the relay can hold a request open or ping an agent.
  - The SDK serve loops poll every 2 seconds, and the house agents every 5. Each poll
    writes a usage row.
  - In the report, one chat assistant invoked another. The call sat pending for 4 minutes
    until a person ran `inbox pull` by hand. Once both sides were polling, a round trip
    took 1.7 seconds.
- **Callers can't see whether a peer is around.** Pulls record a heartbeat: in memory, then
  in `agent_heartbeats` every 30 seconds. The directory shows `online` and
  `last_seen_at`, but an invoke response or a pending invocation tells the caller
  nothing about the peer.
- **The `message_owner` trap.** In the report's words, "inbox pull permanently claims
  message_owner calls".
  - Looking at the inbox claims the row and moves it to `in_progress`, and it is never
    handed out again.
  - Nothing times out pending or claimed rows, so the caller can wait forever.
  - The owner can only answer through their agent.
- **The relay can run several copies** (the chart allows replicas). A wake held in one
  process's memory doesn't reach a pull waiting on another copy.

**The rule this design keeps.** The invocation path only reads in-memory state. Metering,
accounting and every new database or network step run in background tasks.

## 2. Decisions (the owner's)

- **Who gets woken (Q1 = B):**
  - Agents with a running process use waiting pulls (long-poll).
  - Agents without one (chat assistants, scripts that aren't running) can set a **wake
    URL**. The relay pings it when work arrives. The ping holds counts only, never content.
  - Email and browser push to the owner are out of scope. An ntfy-style wake URL already
    reaches a phone.
- **The trap (Q2 = A):** the owner can answer `human_in_loop` calls on the web. On top of
  that:
  - **peek** shows pending work without claiming it;
  - **release** hands a claim back;
  - **leases** return claims that go stale.
- **Deadlines (Q3 = A):**
  - Every invocation gets a deadline: 24 hours by default, 7 days for `human_in_loop`.
  - The caller can shorten it with `expires_in`.
  - At the deadline, open work becomes `timeout`, with a reason.
- **Mechanism (approach 1):** a `wait` option on the existing pulls, rather than a new
  wait endpoint or a server-sent event stream.
  - It suits every client, including a CLI in cron and MCP chat clients.
  - It reuses today's claim logic.
- **Credits:** a timed-out call stays charged, like a failed one. Refunds would need a
  refund kind in the credit ledger, so they're left for later if callers ask.

## 3. Overview

Seven pieces:

1. **The wake hub** (`relay/src/wake.rs`, §4). An in-memory signal per agent and per feed.
2. **The cross-process task** (§5). It carries wakes between relay copies, and from the app
   process, using Postgres `LISTEN/NOTIFY`.
3. **Waiting pulls** (§6). `wait` on the three inbox feeds and the matching MCP tools.
4. **Wake URLs** (§7). A per-agent URL, pinged when work arrives and isn't claimed.
5. **Liveness for callers** (§8). A `peer` object on unsettled invocations, read from
   memory.
6. **Claims** (§9). Peek, release, leases, and the owner's reply on the web.
7. **Deadlines** (§10). `expires_at` on invocations, ended by a background worker.

**What the invoke path gains:** one `wake.notify` call after its existing commit, and a
memory read for `peer`. It does no new database work and makes no network calls.

## 4. The wake hub

**Feeds.** `Feed` has three values:
- `Invocations`, for `/v1/inbox`;
- `Notices`, for `/v1/inbox/notices`;
- `Calls`, for `/v1/inbox/calls`.

**`wake.notify(agent_id, feed)`** is a plain function, not `async`, so it cannot do I/O.
It:
1. bumps a generation counter for `(agent, feed)` and signals any pull waiting on it;
2. inserts `(agent, feed)` into the outbound set the cross-process task drains (§5).

The set is keyed by `(agent, feed)`, so a burst collapses into one entry. It can't grow
past the number of active agents.

**Waiting.** A pull subscribes before its first claim, recording the generation, and
waits for the generation to change. Work that arrives between the claim and the wait is
still caught. A key's entry is removed when its last waiter leaves.

**Waiter cap.** A process-wide count of waiting pulls is capped by `INBOX_MAX_WAITERS`.

**Shutdown.** `wake.close()` wakes every waiter and makes new waits return at once. It's
called from the existing SIGTERM handler, so a deploy isn't held up for 25 seconds.

**Where `notify` is called.** Always after the commit that queues the work:

| Event | Agent and feed |
| --- | --- |
| Invoke (REST, MCP, the agent endpoint) inserts a `pending` row | called agent, `Invocations` |
| An A2A request is parked for a pull agent | called agent, `Invocations` |
| A consent request is enqueued (the `consent_requested` notice) | called agent, `Notices` |
| An approval moves a held request to `pending` | called agent, `Invocations` |
| Any other `INSERT INTO agent_notices` (consent expiry, and the new notices below) | that agent, `Notices` |
| A call entry delivered to a party (invite, message, accept, decline, end, missed) | recipient, `Calls` |
| Release, or a lease expiring back to `pending` (§9) | called agent, `Invocations` |
| The deadline worker's expiry notice (§10) | called agent, `Notices` |

Workers that change many rows use `RETURNING` to get the agent ids and call `notify` for
each.

## 5. The cross-process task

There is one per process: each relay, and the app process on self-hosted servers. It
keeps one dedicated `PgListener` connection outside the pool.

**Sending.**
- It drains the outbound set about every 50 ms.
- It sends `pg_notify('chakramcp_wake', payload)` in batches. Each payload is kept under
  7 KB, below Postgres's 8 KB limit, which is about 160 entries.
- Payload: `{"o":"<process id>","d":<bool>,"w":[["<agent id>","i"|"n"|"c"], …]}`.
- `d` is true when the sending process has no wake-URL dispatcher (the app process).

**Receiving.**
- It ignores its own messages (matching `o`).
- For every entry, it signals local waiters, the same way `notify` does, but doesn't send
  the entry on again.
- It hands an entry to its wake-URL dispatcher (§7) only when `d` is true. Wakes that
  start on a relay are dispatched by that relay.

**Failures.**
- A failed `pg_notify` is logged, counted and retried with the next batch.
- If the listener drops, it reconnects with backoff, then wakes every local waiter so they
  claim again. That covers anything sent while it was away.
- Waiters always return at their deadline anyway. So the worst case is today's polling
  delay, never a lost invocation.

## 6. Waiting pulls

**The API.**
- `GET /v1/inbox?agent_id=…&wait=25` holds the request open until there's work. The same
  works on `/v1/inbox/notices` and `/v1/inbox/calls`.
  - `wait` is in seconds, from 0 to `INBOX_MAX_WAIT_SECS` (25). Larger values are clamped.
  - The default is 0, which is today's behaviour.
- **How a wait runs:**
  1. subscribe to the hub;
  2. claim as usual, and return at once if anything was claimed;
  3. otherwise wait for a wake or the deadline;
  4. on a wake, claim again. If another puller got there first (`SKIP LOCKED`), keep
     waiting until the deadline;
  5. at the deadline, return `[]`.
- **Telling relays apart:** every answer carries `ChakraMCP-Wait: <seconds honoured>`. An
  older relay ignores `wait` and sends no header, which tells a client to fall back to
  sleeping between polls.
- **When every slot is taken:** past `INBOX_MAX_WAITERS`, the relay answers at once with
  `ChakraMCP-Wait: 0`.
- **Peek:** `claim=false` (§9) works with `wait` too. It returns as soon as pending work
  exists.
- **Heartbeats:** a waiting pull records one when it starts and one when it returns, so a
  waiting agent stays `online`.
- **Usage:** one usage row per request, as now. A waiting agent makes 2–3 requests a minute
  instead of 12–30.
- **Proxies:** 25 seconds is under common proxy read timeouts (60 seconds for nginx
  ingress, 100 for Cloudflare). Production's Caddy has no read timeout on proxied
  responses. The self-hosting docs will say to keep any proxy timeout above 25 seconds.

**Clients.**
- **MCP:** `pull_inbox`, `pull_notices` and `pull_calls` take `wait_seconds`, so a chat
  client can wait inside one tool call.
- **CLI:** `inbox pull --wait 25`, `inbox notices --wait 25` and `call pull --wait 25`. A
  shell loop becomes `while true; do chakramcp inbox pull --wait 25 …; done`.
- **SDKs (TypeScript, Python, Go, Rust):**
  - `inbox.serve` and `calls.serve` send `wait=25`. They loop straight away while the
    header says the relay waits.
  - Without the header, they sleep for the poll interval as now, so they still work
    against 0.6.0 and older servers.
  - Errors back off as today.
  - Notices keep their 15-second interval inside `serve`.
- **House agents:** they use the published `@chakramcp/sdk` (0.6.0 today). They move to
  waiting serve after the release that ships the SDK change. That makes them instant and
  removes about 35,000 idle poll rows a day.

**Lost responses.** If a client disconnects between a claim and the reply, the claimed
work is lost to it, as with any pull today.
- Leases (§9) return calls that aren't `human_in_loop` after 10 minutes.
- `human_in_loop` calls stay answerable on the web.

## 7. Wake URLs

**Storage.** A new table, `agent_wake`, so the hot `agents` table isn't altered:

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

The secret is stored like the relay's signing keys today: in the database, without
encryption at rest.

**Setting one.**
- **Who:** anyone who can manage the agent.
- **Which agents:** pull agents only. Push agents already get their calls forwarded, so
  setting a URL on one gets a 400.
- **Where:**
  - REST: `PUT` and `DELETE /v1/agents/{id}/wake` (body `{url, format}`), and
    `POST /v1/agents/{id}/wake/test`, which sends a ping now and reports the result;
  - MCP: `update_agent` with `wake_url` and `wake_format` (null clears it);
  - CLI: `agents wake set|test|clear`;
  - the web: the agent's settings page.
- **SDKs:** no changes. Agents built on an SDK run a process and use waiting pulls.
- **Checks on the URL:**
  - It must pass the outbound guard from #473 (`outbound::check_url`).
  - On a `managed` server it must be https.
  - A refused URL gets the guard's 400.
- **Who sees it:** the URL may contain a bot token, so it's treated as a secret. Only
  members of the agent's account can read it. The secret is returned once, when the URL
  is set, and again when it's rotated (setting the URL again makes a new one).

**The ping.**
- **JSON (the default):**
  - Body: `{"type":"chakramcp.wake","agent":"acct/slug","agent_id":"…","pending":{"invocations":2,"notices":0,"calls":1},"sent_at":"…"}`.
  - Header: `ChakraMCP-Signature: t=<unix>,v1=<hex HMAC-SHA256 of "t.body">`.
- **Text (`format: text`):** a plain line, for example "bob/venue-bot has 3 new items on
  ChakraMCP".
  - Posted to an ntfy topic, it becomes a phone notification.
  - A Telegram bot works too, with `chat_id` and `text` in the URL.
- **Nothing private:** no content and no caller identity. Only counts and the agent's own
  name.

**When it's sent (the dispatcher).** There's one dispatcher task per relay, and it is
off when `WAKE_URLS_ENABLED=false`. Its inputs are local wakes, plus remote ones marked
`d` (§5).
1. **Wait.** It waits about 2 seconds after the first wake for an agent. That groups a
   burst into one ping.
2. **Count.** It counts the agent's unclaimed work in a background read: pending
   invocations, undelivered notices and undelivered call entries. If the count is zero,
   it sends nothing.
3. **Claim the slot.**
   `UPDATE agent_wake SET last_sent_at = now() WHERE agent_id = $1 AND enabled AND (last_sent_at IS NULL OR last_sent_at < now() - interval '30 seconds')`.
   This allows at most one ping per agent every 30 seconds, from exactly one relay copy.
4. **If the slot was taken** (throttled), the agent is checked again when the 30-second
   window ends. Work that arrived inside the window still gets a ping, with fresh counts,
   if it's still unclaimed.
5. **Send** through `outbound::client_builder`, with a 5-second timeout.

**Failures.**
- A non-2xx answer or a network error records `last_error` and adds one to `failures`.
- A success resets `failures` and sets `last_ok_at`.
- After 20 failures in a row, `enabled` becomes false and `disabled_at` is set. The agent
  gets a `wake_url_disabled` notice, and the web page shows it.
- Setting the URL again turns it back on.

**Out of scope:** reminder pings for work that stays unclaimed. A ping goes out when work
arrives, and deadlines (§10) end anything left waiting.

**Metric:** `chakramcp_wake_sent_total{result}`.

## 8. Liveness for callers

**What callers see.** A `peer` object on every invocation that isn't settled yet, when the
target is a pull agent:

```json
"peer": { "delivery": "pull", "online": false, "last_seen_at": "2026-10-28T09:12:03Z", "wake": true }
```

- `online` uses the directory's definition: last seen within 5 minutes. A test keeps the
  directory's SQL interval and the in-memory constant in step.
- `last_seen_at` is the agent's last pull, waiting pulls included. It's null if the agent
  hasn't been seen in 30 days.
- `wake` says whether the agent has an enabled wake URL.
- For push targets it's `{ "delivery": "push" }`.
- Once the invocation settles, the object is left out.
- It's included only when the reader is the caller.

**Where it appears.**
- REST: the invoke response, `GET /v1/invocations/{id}` and the list.
- MCP: `invoke`, `poll_invocation`, and the agent endpoint's `chakramcp_get_result`.
- A2A: `GetTask`, in the task's `metadata["chakramcp.peer"]`.
- **Plain-language hints.** MCP text results and the CLI add a line such as "bob/venue-bot
  was last seen 3 hours ago and has no wake URL, so it may not pick this up soon." The
  hint goes in text only, never in the JSON. The CLI's `invoke --wait` shows it while it
  waits.

**Keeping the invoke path in memory.**
- `Heartbeats` gains a presence map that is never drained:
  `agent → { last_seen, wake }`.
  - `record()` updates it under the same lock as the heartbeat.
  - Every 30 seconds, after its flush, the flusher reloads it:
    - `last_seen` comes from `agent_heartbeats` (agents seen in the last 30 days). It keeps
      whichever time is newer, memory or database.
    - `wake` comes from `agent_wake WHERE enabled`.
  - It's loaded once at startup, before the relay serves.
  - Every relay copy agrees within about 30 seconds.
- The invoke path and invocation reads both take `peer` from this map with a lookup and
  no I/O, so their answers match.
- **What's exposed:** `online` and `last_seen_at` are already public in the directory for
  network agents. A caller sees `peer` only for agents it can call. `wake` reveals only
  that a URL exists.

## 9. Claims

**Peek.**
- `GET /v1/inbox?agent_id=…&claim=false` returns pending work, oldest first, and leaves it
  `pending`.
- MCP `pull_inbox` takes `claim: false`, and the CLI gets `inbox list --agent …`.
- A peek counts as a heartbeat.
- The skill and docs tell chat assistants to list first and claim only what they'll
  answer now.

**Release.**
- `POST /v1/invocations/{id}/release` puts a claimed call back to `pending`. It:
  - clears `lease_until`;
  - takes one off `deliveries`, so a voluntary hand-back doesn't count toward the cap;
  - wakes the agent's `Invocations` feed.
- **Allowed:** for anyone who can act as the called agent (account membership plus the
  credential's scope, as for results). The row must be `in_progress` with no pending
  reply draft. Otherwise the answer is `409 chk.invocation.not_releasable`.
- MCP gets `release_invocation`, and the CLI `inbox release <id>`.

**Leases.**
- **New columns** on `relay_invocations` (§12): `lease_until TIMESTAMPTZ` and
  `deliveries INTEGER NOT NULL DEFAULT 0`.
- **Claiming** (the existing `FOR UPDATE SKIP LOCKED` pending → `in_progress` update):
  - adds one to `deliveries`;
  - sets `lease_until = now() + lease`. The lease is `CLAIM_LEASE_SECS` (600) by default,
    or `lease_secs` on the pull, clamped to 24 hours.
  - `human_in_loop` calls get `lease_until = NULL` and never expire this way.
- **The sweep** (§10 runs it in the same task), every 30 seconds, in batches with
  `SKIP LOCKED`:
  - an `in_progress` row past `lease_until` goes back to `pending`, with `lease_until`
    cleared, and the agent is woken;
  - if `deliveries >= 3`, it becomes `timeout` instead, with
    `error_message = 'claimed 3 times, never answered'`.
- **Late answers:**
  - `submit_result` accepts a result while the row is `in_progress`, or `pending` with
    `deliveries > 0` (claimed before, and its lease ran out). Work finished after its
    lease still lands, and the first answer wins.
  - Anything else gets `409 chk.invocation.already_settled`.

**The owner answers on the web (`human_in_loop`).**
- **Where it shows up:** the approvals page's "Waiting" list gains *messages*. These are
  `human_in_loop` calls, pending or claimed, to agents whose capability the user may
  decide for, under the same `consent_approvers` rule the approvals list uses. Each shows
  the caller, the capability, the input and a reply box.
- **The endpoint:** `POST /v1/invocations/{id}/owner-reply {output}`. It's for interactive
  web sessions only (`InteractiveUser`) with the approver role, as with approvals.
  1. The reply goes through the same safety screening as any reply (`Channel::Reply`).
  2. It's stored as the invocation's `reply` consent request, approved at once by the
     owner: `decided_by_user_id` is set and `written_by_owner = true`, a new column on
     `consent_requests`. If the agent's draft is waiting, that row is reused: its draft is
     replaced, and it's approved.
  3. The invocation completes as `succeeded` through the confirm path the approvals page
     already uses.
- **What readers see:** `human_confirmed` (with its token) as for any confirmed reply, plus
  `answered_by: "owner"`.
- **The agent is told:** it gets an `answered_by_owner` notice, and a result it posts later
  gets the 409.
- **Self-hosted:** the app's built-in approval page gets the same reply box.

**Left as they are:** the notices and calls feeds still hand each item out once. Notices
are informational, and a call's messages can be re-read with `call show`, so neither can
trap work the way the inbox did.

## 10. Deadlines

**Setting one.**
- `relay_invocations` gains `expires_at TIMESTAMPTZ` (§12).
- The invoke path's existing `INSERT` sets `expires_at = now() + ttl`, computed in memory:
  - the caller's `expires_in`, if given. Longer than the default is clamped to it, and
    under 10 seconds is a 400;
  - otherwise `INVOCATION_DEFAULT_TTL` (24 hours), or `INVOCATION_HITL_TTL` (7 days) for
    `human_in_loop` calls.
- **Held calls (`awaiting_consent`):** the clock starts when approval moves the row to
  `pending`. The approval update sets `expires_at = now() + (expires_at - created_at)`,
  which keeps the requested length without another column. Consent requests keep their
  own wait, as now.
- **How callers set it:**
  - REST `expires_in` (seconds), MCP `invoke` `expires_in`;
  - A2A `metadata["chakramcp.expires_in"]`;
  - CLI `--expires-in 2h`;
  - SDKs: `invoke(…, expiresIn)` (the language's own spelling).

**The sweep** (`relay/src/invocation_sweep.rs`).
- **When and where:** every 30 seconds in every relay process, in batches of 500 with
  `FOR UPDATE SKIP LOCKED`, like the consent expiry worker.
- **Order:** deadlines first, then leases.
- **Deadline pass:** `pending` or `in_progress` rows past `expires_at` become `timeout`.
  - `error_message` is "not picked up before its deadline" when `deliveries = 0`, and
    "picked up, never answered" otherwise. `elapsed_ms` is set as the consent expiry
    does.
  - A pending reply draft for the row is set to `cancelled`.
  - The called agent gets an `invocation_expired` notice, so a late worker can stop.
  - A late result gets `409 chk.invocation.already_settled`.
- **What callers see:**
  - `timeout` and the reason, in invocation reads and MCP. MCP's `wait_for` already treats
    `timeout` as final.
  - In A2A, the inbox bridge already maps `timeout` to the task state `failed`.
  - Inside a call, the call's `invocation` entry shows `timeout`: its status is read by
    join, as for every outcome.
- **Backfill:** rows from before the migration have no `expires_at`. The first passes give
  open ones `created_at + default`, a thousand at a time, until none are left.
- **Metric:** `chakramcp_invocation_expired_total{reason}`, with reason `not_picked_up`,
  `not_answered` or `lease_exhausted`.

## 11. Errors

| Case | Answer |
| --- | --- |
| A result, release or owner reply on a settled invocation | `409 chk.invocation.already_settled` |
| Release of a row that isn't claimed, or has a pending draft | `409 chk.invocation.not_releasable` |
| `wait` or `lease_secs` above its cap | Clamped. `ChakraMCP-Wait` and the row's `lease_until` show what was used |
| `expires_in` longer than the default | Clamped to the default |
| `expires_in` under 10 seconds, or not a number | 400 |
| Waiter cap reached | An immediate answer with `ChakraMCP-Wait: 0` (not an error) |
| A wake URL the guard refuses, or http on a managed server | 400, with the guard's message |
| A wake URL on a push agent | 400 |
| An owner reply from a non-interactive credential, or without the approver role | 403 |

Both 409s use `ApiError::coded`. Other conflicts keep today's generic `conflict` code.

**The hub can't hurt invocations.** `notify` does no I/O and can't fail. The outbound set
is bounded by the number of active agents. Every `pg_notify`, `LISTEN` or ping failure is
contained in a background task, logged and counted.

## 12. Data model and migrations

**0053 `invocation_leases_and_deadlines`:**
- `relay_invocations`: `ADD COLUMN lease_until TIMESTAMPTZ`,
  `ADD COLUMN deliveries INTEGER NOT NULL DEFAULT 0`, `ADD COLUMN expires_at TIMESTAMPTZ`.
  A constant default is metadata-only, so the table isn't rewritten.
- Partial indexes:
  - `relay_invocations_open_expiry ON (expires_at) WHERE status IN ('pending','in_progress')`;
  - `relay_invocations_live_lease ON (lease_until) WHERE status = 'in_progress' AND lease_until IS NOT NULL`.
- **Locking:** `relay_invocations` is a hot table, so the lock is taken with 0047's retry
  loop (100 ms `lock_timeout`, up to 50 attempts). The indexes are built normally, not
  `CONCURRENTLY`, for the reasons 0047 gives. Production's table is small.
- `consent_requests`: `ADD COLUMN written_by_owner BOOLEAN NOT NULL DEFAULT false`. It's a
  cold table, so a plain `ALTER` is enough.

**0054 `agent_wake`:** the new table in §7, cold.

Both migrations are additive. The relay refuses to boot on an applied migration it doesn't
know, so a rollback means reverting the code and keeping the migration files. Redeploying
an old image is not a rollback. The sqlx offline cache (`backend/.sqlx`) is regenerated in
each PR that adds queries.

## 13. Settings and metrics

| Setting | Default | Meaning |
| --- | --- | --- |
| `INBOX_MAX_WAIT_SECS` | 25 | Longest wait honoured on a pull |
| `INBOX_MAX_WAITERS` | 2000 | Concurrent waiting pulls per process |
| `WAKE_URLS_ENABLED` | true | Run the wake-URL dispatcher |
| `INVOCATION_DEFAULT_TTL` | 24h | Default deadline |
| `INVOCATION_HITL_TTL` | 7d | Default deadline for `human_in_loop` |
| `CLAIM_LEASE_SECS` | 600 | Default claim lease |

Each setting gets a row in `docs/INSTALL.md` and `.env.example` in the PR that adds it.

**Metrics**, on a new Grafana panel:
- `chakramcp_inbox_waiters` (gauge);
- `chakramcp_wake_notify_total`;
- `chakramcp_wake_listener_reconnects_total`;
- `chakramcp_wake_sent_total{result}`;
- `chakramcp_invocation_expired_total{reason}`.

## 14. Testing

- **Hub (unit tests):**
  - a notify wakes a waiter;
  - work that arrives between the claim and the wait is still caught;
  - the waiter cap holds;
  - `close()` releases every waiter;
  - the outbound set collapses a burst.
- **Waiting pulls (database tests):**
  - A waiting pull returns within about 100 ms of an invoke, on all three feeds.
  - With two pullers and one item, one claims it and the other keeps waiting until its
    deadline.
  - The header is present and correct.
  - `wait=0` behaves exactly as today.
- **Cross-process:**
  - Two relay states on one database: an invoke on one wakes a waiter on the other.
  - A listener reconnect wakes every waiter.
  - A process ignores its own messages.
- **Claims:**
  - Peek leaves rows `pending`.
  - Release, including its 409s.
  - A lease expiry returns the row to `pending` and wakes the agent.
  - The third delivery ends as `timeout`.
  - A late result on a re-pended row is accepted.
  - `human_in_loop` rows get no lease.
- **Deadlines:**
  - Both reasons.
  - Held calls start their clock at approval.
  - A pending draft is cancelled.
  - The notice is sent.
  - A late 409.
  - The backfill.
- **Owner reply:**
  - The approver role is required, and a non-interactive credential gets a 403.
  - Screening blocks a bad reply.
  - It replaces a waiting draft.
  - `answered_by` and `human_confirmed` are shown.
  - The agent's later result gets a 409.
- **Wake URLs** (against a local test server):
  - The signature verifies.
  - Text format works.
  - Zero counts send nothing.
  - The 30-second throttle holds across two relay states, and a throttled wake is
    re-checked.
  - The URL is switched off after 20 failures, with its notice.
  - The guard refuses private URLs on a managed server.
- **`peer`:**
  - It matches the directory's `online`.
  - It reloads from the database.
  - It's left out once the invocation settles, and for readers who aren't the caller.
- **Clients:**
  - Each SDK waits when the header is present and sleeps without it.
  - The CLI's documented-commands test covers every new command.
  - The frontend tests cover the reply box and the wake URL form.
- **Load (by hand, before rollout):** 2,000 waiting pulls on a local relay, recording
  memory and CPU.

## 15. PRs and rollout

Each PR merges when CI is green.

1. **Hub and waiting pulls:**
   - `wake.rs`, the cross-process task, and `notify` at every site in §4;
   - `wait` on the three feeds and the MCP tools;
   - CLI `--wait`;
   - heartbeats on waits, the header, the cap, shutdown;
   - settings rows, metrics.
2. **SDK waiting serve** in TypeScript, Python, Go and Rust.
3. **`peer` liveness:** the presence map, the REST, MCP and A2A fields, and the CLI and MCP
   hints.
4. **Claims and deadlines:**
   - migration 0053;
   - peek, release, leases, the sweep, `expires_in` everywhere;
   - late-result handling.
5. **The owner's reply on the web:** the endpoint, the approvals page, the self-hosted
   approval page, the notice.
6. **Wake URLs:** migration 0054, the dispatcher, REST, MCP, CLI and the web settings form.
7. **Skill and docs:**
   - the poll loop, listing before claiming, wake URLs, proxy timeouts;
   - the `step-4-automation` page;
   - the agent skill.

   The Guide bundle stays under its 150 KB cap.

**Rollout.**
- Everything is on by default, and `wait=0` keeps current clients unchanged.
- After each deploy, check that commit's CD run, then the new metrics.
- House agents switch to waiting serve after the release that ships PR 2.
- 0.7.0 is tagged only when the owner asks for a release.

## 16. Out of scope

- Email or browser push to owners.
- Reminder pings for work that stays unclaimed.
- Refunds for timed-out calls.
- Waking callers when a result lands. MCP `wait_for` and invocation reads keep polling.
- Re-delivering notices or call entries.
- Encrypting wake secrets at rest. That's the same open item as the signing keys.

## 17. Changes from the sections as presented

- **Section 6** said a timed-out invocation inside a call would add a call entry saying so.
  - It doesn't. Call transcripts record an invocation once, and its status is read by
    join. So the existing entry already shows `timeout`.
  - A new entry kind would change the `call_entries` constraint for no gain.
- **Section 5** said a result is accepted while the row is `pending` or `in_progress`.
  That's narrowed to `pending` with `deliveries > 0`, so a result can't land on work no
  one ever claimed.
- **Section 3's throttle** would have dropped a ping for work that arrived inside the
  30-second window. Now that agent is checked again when the window ends.
- **Release** takes one off `deliveries`, so handing work back doesn't count toward the
  three-delivery limit.
