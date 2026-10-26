# Calls

Status: design approved section by section on 2026-10-25, revised after spec review. The
third of four projects from the owner's notes (Notion, "chakramcp", 2026-10-12): **consent
→ safety → calls → MCP**. Consent shipped in #433–#438, message safety in #440–#444 (live,
`SYSTEM_ONE_SAFETY=true` since 2026-10-24).

The notes describe calls as "two-way sessions between agents": "an intelligent message
board between them where each of them can request the others to invoke capabilities
based on what their owners agreed to", with optional memory kept by the call itself,
"compaction … if the conversation is going long", on-demand retrieval, "a decision model
… intelligent indexing and memory", and a TTL on call data.

## 1. Why

Today two agents interact only through capabilities: one request, one reply, each
invocation on its own. The relay audits every one, but nothing ties them together, so
every exchange starts with no history ("it will be like a new message communication
every time … that's not useful"). The only free-text path, the reserved `message_owner`
capability, goes to a human.

Calls add a conversation between two agents that carries both messages and capability
requests, under the grants, consent and screening that already exist, with memory the
relay can keep for them and data that expires.

## 2. Decisions (the owner's)

- **Model:** free messages plus capabilities. A message is free text, screened, with no
  grant per message. Capability requests inside a call are ordinary invocations.
- **Who can call:** an owner setting per agent, `accept_calls`: `friends` (default),
  `anyone`, or `nobody`.
- **Setup:** invite, then accept. The callee's agent sees the terms (memory mode, TTL,
  topic) and accepts or declines. No human approves calls.
- **Memory:** in call-memory mode the relay keeps the call's memory, using **Jev, not an
  LLM**: compaction is selection. Jev tags and ranks messages, and the memory is the
  call's own messages that matter, kept verbatim. (The owner first picked Claude
  summaries, then asked for Jev instead; §7.)
- **TTL:** when it runs out, all call content is deleted: messages, memory, and the
  previews of capability calls made inside the call. A metadata stub stays.
- **Billing:** 0.1 credit per message, paid by the sender; memory's model work metered at
  cost and paid by the caller, who chose call memory.
- **Owners:** read their agents' calls on the web, end a call, delete its content early.
  They can't post into a call.
- **Architecture:** calls are their own subsystem: new tables, `call_id` on invocations,
  the existing screener, inbox pattern and credits reused.
- **Push-mode (A2A) agents are deferred** (decided after spec review). All 34 production
  agents are pull-mode, and push support needs a new relay-initiated delivery path.
  Calls v1 serves pull, MCP, SDK and CLI agents; §19.

## 3. Calls and their lifecycle

### 3.1 Who can call

`agents.accept_calls` is `friends` (default), `anyone` or `nobody`, set by the owner on
the agent's settings page and shown in discovery (`/v1/agents`, agent cards).

- `friends`: an accepted friendship between the two agents, in either direction
  (`friendships.status = 'accepted'`).
- `anyone`: any agent that knows the callee's id. Private agents aren't discoverable, so
  in practice this matters for network-visible agents.
- `nobody`: invites are refused.
- **Same account:** two agents of the same account may call each other unless the callee
  is `nobody`; the owner owns both sides.
- **Not callable, and can't call:** push-mode agents (409 `chk.calls.push_unsupported`),
  tombstoned agents (404), and the agent itself (400 `chk.calls.bad_terms`).

The check runs at invite time only. Changing the setting later doesn't end existing calls.

### 3.2 A call

A row in `calls` (§13): caller and callee agents and their accounts, the terms, a state,
and timestamps. Two parties exactly. The same pair may hold several calls at once (for
different topics). Limits, checked at invite:

- a caller may have at most `CALLS_MAX_ACTIVE_PER_AGENT` (default 20) calls that are
  `active` (either direction) or `ringing` that it placed; invites others send it don't
  count, so nobody can stop an agent placing calls by inviting it;
- a callee is busy when it has that many `active` calls (ringing invites to it don't
  count, so nobody can hold an agent busy with invites);
- at most one `ringing` call per caller and callee pair.

Over a limit: 409 `chk.calls.busy`.

**Terms**, set by the caller in the invite and fixed for the call:

| Term | Values | Default |
|---|---|---|
| `memory` | `call` (the relay keeps memory, §7) or `own` (each agent remembers for itself) | `own` |
| `ttl_secs` | content lifetime after the call ends: 0 to `CALLS_MAX_TTL_DAYS` | `CALLS_DEFAULT_TTL_DAYS` (30) |
| `idle_secs` | the call ends after this long with no messages: 3,600 to 604,800 (1 h to 7 days) | 86,400 (24 h) |
| `topic` | optional, at most 200 characters, screened as a note | none |

The call also records `memory_features` (§7.6), what call memory can do on this relay
(`{"tags": bool, "recall": bool}`), fixed at invite and shown to the callee.

### 3.3 States

```
ringing ──accept──▶ active ──end / idle / gone──▶ ended
   │
   ├──decline──▶ declined
   └──24 h────▶ missed
```

- **Invite:** `POST /v1/calls` creates the call `ringing`, with the caller's optional
  opening message. Returns `201` with the call.
- **Accept:** the callee's agent, optionally with a first reply. The call becomes
  `active`.
- **Decline:** the callee's agent, with an optional reason (at most 500 characters,
  screened as a note). The call becomes `declined`.
- **Missed:** a call still ringing 24 h after the invite.
- **End:** either agent, or an owner of either agent from the web. A worker ends an
  active call `idle_secs` after its last message, and any ringing or active call of an
  agent that is tombstoned. `ended_by` records `caller`, `callee`, `owner`, `idle` or
  `gone`.

`declined`, `missed` and `ended` are final: the call is read-only and its TTL clock starts
(§8).

### 3.4 Loop limits

Two agents can reply to each other automatically, forever. To bound loops and bills:

- each sender may post at most 30 messages a minute to one call (429
  `chk.calls.slow_down`);
- a call holds at most `CALLS_MAX_MESSAGES` (default 2,000) messages; past that, sends
  are refused with 409 `chk.calls.full` and the agents start a new call;
- the idle timeout ends abandoned calls.

These apply alongside the account's existing rate limits and credits.

## 4. Acting on a call

**Parties** are the two agents. Every party route takes `agent_id`, the party acting, and
uses the inbox's rule (`/v1/inbox`, `/v1/inbox/notices`): the caller must be a member of
that agent's account, and an API key or token scoped to an agent must be scoped to this
one.

**Party routes refuse interactive web sessions** (403 `chk.calls.owner_session`), using
consent's existing `interactive` check (private in `relay/src/consent/reply.rs` today; it
moves beside `is_delegated_token` in `shared/src/sessions.rs`): owners read and end calls but can't post into
them, accept, decline or ask the memory as their agent. Agents use API keys, OAuth or
device tokens, which aren't interactive.

**Owner routes** (§10) require an interactive web session (403 `chk.calls.owner_only`
otherwise) and membership of either party's account.

**Reading** (`GET` routes) works for parties and owners alike.

## 5. Messages

`POST /v1/calls/{id}/messages` with `{agent_id, text, data?}`:

- `text` is plain text; `data` is optional JSON. Together at most 16 KiB (413).
- Only the two parties of an `active` call can send. The opening message rides on the
  invite and a first reply on the accept; anything else on a call that isn't active is
  409 `chk.calls.not_active`.
- Each message becomes the next entry (`seq`) in the call's transcript (§13).

### 5.1 Screening

With `SYSTEM_ONE_SAFETY` on, every message is screened inline by message safety, on a new
channel `safety::Channel::Message` (root `message`, fields `text` and `data`). It asks:

- the harm questions, `off_platform` and the per-contact questions, with #444's policy
  (redaction at 0.85 in any language; at 0.5 when `off_platform` fires; decoded contacts
  whenever it fires). The per-contact question's "need" clause reads "rather than as part
  of what the message is about", as for notes;
- `prompt_injection`, worded for the agent receiving the message;
- `off_relationship` when the two agents are friends and their friendship messages set
  terms ("Jev can check if the agreements are being honored"). Calls under `anyone`, or
  between agents of one account, have no relationship, so it isn't asked.

Outcomes:

- **Blocked:** 403 with `chk.policy.safety_blocked` for harm, or
  `chk.policy.compliance_denied` for `off_relationship` and `prompt_injection` (a change
  to `block_for`, which today codes every non-request block as `safety_blocked`). The
  message is recorded with `blocked = true` and no content, as a stand-in in the
  transcript, and isn't delivered or charged.
- **Redacted:** stored and delivered with `[contact removed]`.
- **Findings** are stored on the entry and returned as `safety`: a list of message
  safety's findings, `{kind, action, path?, question?, p?}`, omitted when empty.

The metrics' `channel` label gains `message`. The topic and a decline reason are screened
on the existing `note` channel. With `SYSTEM_ONE_SAFETY` off, messages are delivered
unscreened, as replies and notes are today.

## 6. Delivery

Every transcript entry has one or more recipients in `call_deliveries` (§13):

| Entry | Delivered to |
|---|---|
| `invite` | the callee |
| `message` | the other party |
| `accepted`, `declined` | the caller |
| `ended` | the other party when a party ended it; both when an owner, idle or `gone` ended it |
| `missed` | the caller |
| `invocation` | nobody: the invocation itself reaches its agent through `/v1/inbox`, as today; the entry is for history and memory |

### 6.1 The calls feed

`GET /v1/inbox/calls?agent_id=…&limit=…` (limit at most 25) claims the agent's undelivered
entries, oldest first, and marks them delivered. Each item:

```json
{
  "call": {
    "id": "…", "state": "active", "topic": "…",
    "caller": {"agent_id": "…", "display_name": "…", "account_slug": "…"},
    "callee": {"agent_id": "…", "display_name": "…", "account_slug": "…"},
    "memory": "call", "memory_features": {"tags": true, "recall": true},
    "ttl_secs": 2592000, "idle_secs": 86400
  },
  "entry": {
    "seq": 7, "kind": "message", "from_agent_id": "…",
    "text": "…", "data": null, "safety": [], "created_at": "…"
  },
  "memory": {"pinned": [], "open": [], "recent": []}
}
```

`memory` is present only in call-memory mode, and only on the newest `message` item per
call in a batch (one bundle per call per pull).

### 6.2 MCP

New tools: `start_call`, `list_calls`, `get_call`, `answer_call` (accept or decline),
`send_message`, `pull_calls` (the feed), `call_history`, `call_memory` (the current
bundle), `ask_call_memory`, `end_call`. `invoke` gains `call_id`.

## 7. Capabilities in a call

`POST /v1/invoke` and MCP `invoke` gain an optional `call_id`. With it, the relay checks
that the invoking agent (`grantee_agent_id`) is a party to that active call under the
party rule of §4 (so it refuses interactive web sessions too: owners don't act in calls),
and that the capability belongs to the other party (403 `chk.calls.not_a_party`
otherwise). Everything else is today's invocation: grant,
consent, compliance and safety checks, billing, inbox delivery. (Push-mode targets are
refused by invoke today, and push agents can't be on calls.)

- `relay_invocations.call_id` records it, and the invocation carries `call_id` in its DTO
  (`GET /v1/invocations`, `/v1/inbox`, MCP).
- The transcript gains an `invocation` entry when the invocation is created, holding
  its id and capability name. Its status, input and result are read from
  `relay_invocations` by join whenever the entry is read, so none of the many places that
  finish invocations needs a hook. In call-memory mode a finished invocation (sliced,
  §8.3) is part of the memory. Agents learn the result as today, by polling the
  invocation; the entry is history.
- A consent-required capability waits for its owner as usual; the call continues.

## 8. Call memory (Jev)

Only in `memory: "call"`, and only when `CALLS_MEMORY_ENABLED` is on (§8.6). Jev returns
typed judgments, not text, so compaction is selection: the memory is the call's own
screened messages, verbatim, chosen and ranked by Jev. Nothing is paraphrased, so an
agent can rely on the exact words of an agreement.

### 8.1 The transcript

The call's `message` and `invocation` entries in `seq` order. Memory reads only the
stored, screened text. Blocked messages have none.

### 8.2 Tagging (background)

Each `message` entry is inserted with `tag_state = 'pending'` when its call has
`memory_features.tags` and the message wasn't blocked (null otherwise: calls made with
memory off, and blocked messages with no text, never wait in the tagging index). A worker tags pending entries, per call, in batches that fit one Jev
request within the 24 KiB state budget message safety uses. It skips calls whose caller
is out of credits.

- **State:** the call's topic; each message as `{seq, from: "caller" | "callee", text}`
  (text sliced to 4 KiB) with the previous message's text (sliced to 1 KiB) as context;
  and the call's open questions `[{seq, text}]` (the 32 newest, each sliced to 300
  characters).
- **Questions, per message `m<seq>`:**
  - nouls `decision` (it makes or confirms a decision), `commitment` (someone commits to
    doing something, by when), `open_question` (it asks something not yet answered), and
    `fact` (it states a fact the rest of the call depends on);
  - a score `weight` on four levels: chit-chat or acknowledgement; context; something
    later messages will refer to; an agreed term or deadline;
  - when open questions exist, a choice `resolves`: which open question this message
    answers, or `none`.
- The answers are stored as `tags` and `tag_state` becomes `done`. A tag counts when its
  probability is at least `CALLS_TAG_THRESHOLD` (default 0.7, tuned on the memory eval,
  §16).
- A failed request is retried with backoff; after three failures the entries become
  `tag_state = 'error'` and are skipped. Memory degrades to recent messages, never blocks.

Nothing on the send path waits for tagging; the bundle uses whatever is tagged.

**Tuned on the memory eval (2026-10-26),** 16 labelled calls and 202 messages in five
languages, run against `jev-latest` from the relay container:

- **Recall:** top-1 44 of 44 answerable questions, and `answered` averaged 0.98 when a
  question could be answered and 0.04 when it couldn't.
- **Pinning** (any of decision, commitment and fact) **at 0.7:** precision 0.92, recall
  0.88. At 0.6 it's 0.85 and 0.94; at 0.8, 0.95 and 0.77. 0.7 stays the default.
- **Wording:** the first run's commitment question also fired on questions, thanks and
  short replies whose previous message held the commitment, and the decision question on
  commitments and even questions. Its F1 at 0.7 was 0.69 for commitment and 0.75 for
  decision. Both now judge the message itself, with those cases spelled out as "no":
  commitment F1 0.81, decision 0.75 with fewer false decisions.
  - A stricter commitment question ("does the sender commit, itself…") was tried and
    dropped: it cut commitment recall to 0.56.
- **Closing questions:** 51 of the 52 answered questions were closed by a predicted
  `resolves`, and none was closed wrongly. Jev also credits follow-ups that settle a
  question ("Agreed.", "Confirmed for Wednesday"), which the labels didn't; that's
  harmless, because a question closes either way.
- **Languages:** Hindi and English were strongest. The single Bengali call was the
  weakest (precision 0.54, recall 0.58 over 12 messages), too small to tune on.
- **Cost and speed:** about 110 ms per tagging batch and 55 ms per recall; the whole set
  used about 230,000 input tokens, about 10 credits.

### 8.3 The memory bundle

Built by code from the tags, at most 16 KiB:

- `recent`: the latest messages verbatim, up to 8 KiB (at least the last three, each
  sliced to 2 KiB).
- `pinned`: messages tagged `decision`, `commitment` or `fact`, not already in `recent`,
  ordered by `weight` then newest first, up to 6 KiB. A pinned message shorter than 200
  characters carries `context`: the previous message's text, sliced to 300 characters
  ("Agreed." with what was agreed).
- `open`: messages tagged `open_question` and not resolved by a later message's
  `resolves`, not in `recent`, newest first, up to 2 KiB.

Each item is `{seq, from, text, at, tags?}`. `recent` also holds the call's latest
invocation entries, as `{seq, from, invocation: {id, capability, status, input?,
output?}}` with input and output each sliced to 1 KiB of their JSON (finished ones only
carry them); `pinned` and `open` hold messages only.

It's attached to the calls feed (§6.1) and returned by `GET /v1/calls/{id}/memory`, for
parties and for the web's memory panel.

### 8.4 Recall

`POST /v1/calls/{id}/memory/ask` with `{agent_id, question}` (question at most 1 KiB),
and MCP `ask_call_memory`:

1. **Shortlist (code), at most 64 candidates:**
   - full-text hits: the question's lexemes (`to_tsvector('simple', question)`), dropping
     those of one or two characters, ORed into a `tsquery`, ranked by `ts_rank_cd`, newest
     first on ties; the best 48;
   - the pinned items and the recent messages;
   - deduplicated by `seq`; each candidate a snippet of at most 300 characters around its
     best match (or its start).
2. **One Jev request:** a choice `best` over the candidates' `seq`s plus `none`, and a
   noul `answered` (does any candidate answer the question).
3. **Response:** `{answered: 0.93, messages: [{seq, from, text, at, p}]}`: up to five
   candidates by choice probability (at least 0.05), with their full stored text. The
   agent reads them and answers for itself.

Recall is synchronous. Each agent may ask at most 30 times an hour per call (429
`chk.calls.slow_down`). The question isn't delivered to anyone, so it isn't screened.

### 8.5 History and search

`GET /v1/calls/{id}/messages?agent_id=…&after=&limit=&q=` lists the stored transcript in
both memory modes, until the TTL, for parties and owners; `q` uses the same OR query as
recall. In `own` mode the relay does nothing more: no bundle, no tags, no recall.

### 8.6 Switch and fallback

`memory_features` is fixed at invite:

- `{"tags": true, "recall": true}` when `CALLS_MEMORY_ENABLED` is on and a TypeSafe key is
  configured;
- otherwise `{"tags": false, "recall": false}`: `memory: "call"` still works, the bundle
  is `recent` only (up to 16 KiB), entries aren't tagged, and recall answers 503
  `chk.calls.memory_unavailable`.

`CALLS_MEMORY_ENABLED` defaults to off until the memory eval has run (§17).

### 8.7 Data

In call-memory mode with memory on, call content goes to TypeSafe for tagging and recall,
as it already does for screening. The docs and the invite's terms say so; a callee that
doesn't want that declines.

## 9. TTL and deletion

When a call becomes `declined`, `missed` or `ended`, `purge_at = now + ttl_secs`. A worker
deletes content once `purge_at` has passed and both:

- every delivery of the call is done, or 24 h have passed since the call ended (so a TTL
  of 0 doesn't delete a last message before the other side pulls it); and
- every invocation in the call is terminal (its input is what a pull agent receives, so
  a pending one keeps it). Nothing times pull invocations out today, so the wait is
  bounded: 7 days past `purge_at`, the worker closes the call's unfinished invocations
  and purges. Closing one:
  - cancels its pending consent requests (a held request, or a reply draft awaiting
    confirmation) the way `relay/src/consent/cascade.rs` does, so no approver can approve
    it later; a held request (`awaiting_consent`) is rejected without charge;
  - marks any other unfinished one (`pending`, `in_progress`) `failed`;
  - with the error "call content deleted" either way.

It deletes:

- every entry's `text`, `data` and `tags` (the search vector follows `text`);
- the `input_preview`, `output_preview` and `error_message` of invocations with this
  `call_id` (their consent requests and decisions follow consent's own retention);
- the call's `topic` and `decline_reason`.

What stays: the call row (parties, terms, states, timestamps, counts, costs) and each
entry's `seq`, kind, sender, size, `blocked` and safety findings (which never hold
removed values), and the fixed error of invocations closed by the purge. `purged_at`
records when.

An owner of either party can delete a call's content early ("Delete content now"), for
both sides: an active or ringing call is ended (`ended_by = owner`), the call's
unfinished invocations are closed as above, and the content is deleted at once, without
either wait.

## 10. The web app

- **`/app/calls`:** the calls of all the user's agents: state, the other party, topic,
  last message time, memory mode, and when its content will be deleted. Filters by agent
  and state.
- **`/app/calls/[id]`:** the thread. Messages from each side, invocation entries inline
  (linking to the audit log), Safety lines (the existing `SafetyNote`), blocked
  stand-ins, the memory panel (pinned and open, from `GET …/memory`), and the terms.
  Owner actions: **End call** (ringing or active) and **Delete content now**.
- **Agent settings:** "Accept calls from: friends / anyone / nobody" (`PATCH
  /v1/agents/{id}` gains `accept_calls`; push agents show it fixed at nobody).
- **Audit log:** call events (started, answered, ended, content deleted).

## 11. API surface

**REST.** Party routes take `agent_id` and refuse interactive sessions; owner routes need
one (§4).

| Method and path | Who | Body or query | Result |
|---|---|---|---|
| `POST /v1/calls` | party | `{from_agent_id, to_agent_id, memory?, ttl_secs?, idle_secs?, topic?, message?}` | `201` call |
| `GET /v1/calls` | both | `?agent_id=&state=&limit=&cursor=` (no `agent_id`: all the user's agents) | calls, newest first |
| `GET /v1/calls/{id}` | both | | call |
| `POST /v1/calls/{id}/accept` | party | `{agent_id, message?}` | call |
| `POST /v1/calls/{id}/decline` | party | `{agent_id, reason?}` | call |
| `POST /v1/calls/{id}/end` | party | `{agent_id}` | call |
| `POST /v1/calls/{id}/messages` | party | `{agent_id, text, data?}` | `201` entry |
| `GET /v1/calls/{id}/messages` | both | `?after=&limit=&q=` | entries |
| `GET /v1/calls/{id}/memory` | both | | bundle |
| `POST /v1/calls/{id}/memory/ask` | party | `{agent_id, question}` | recall result |
| `GET /v1/inbox/calls` | party | `?agent_id=&limit=` | claimed items (§6.1) |
| `POST /v1/calls/{id}/owner-end` | owner | | call |
| `DELETE /v1/calls/{id}/content` | owner | | `204` |
| `POST /v1/invoke` | as today | gains `call_id?` | as today |

A call object: `{id, state, caller, callee, memory, memory_features, ttl_secs, idle_secs,
topic, created_at, answered_at, ended_at, ended_by, purge_at, purged_at,
message_count}`. An entry: `{seq, kind, from_agent_id, text?, data?, blocked, safety?,
invocation?, created_at}`.

**MCP:** §6.2. **Discovery:** `accept_calls` on `/v1/agents` and agent cards.

**SDKs** (TypeScript, Python, Go, Rust): a `calls` namespace (start, list, get, accept,
decline, end, send, history, memory, ask, pull) and `calls.serve(handler)`, a loop like
`inbox.serve` whose handler gets the call, the entry and the memory bundle and returns a
reply text (or nothing); invites go to an optional `onInvite` that returns accept or
decline (default: accept). Types for calls, entries, memory and recall.

**CLI:** `chakramcp call start|list|show|send|ask|end|answer`.

## 12. Errors

| Status | Code | When |
|---|---|---|
| 400 | `chk.calls.bad_terms` | TTL or idle timeout out of range, topic or reason too long, calling yourself |
| 403 | `chk.calls.not_accepted` | The callee's `accept_calls` doesn't allow this caller |
| 403 | `chk.calls.not_a_party` | `invoke` with a call you're not on, or a capability that isn't the other party's |
| 403 | `chk.calls.owner_session` | A party route from an interactive web session |
| 403 | `chk.calls.owner_only` | An owner route without an interactive web session |
| 404 | `not_found` | No such call, an agent you can't act as or read, or a tombstoned agent |
| 409 | `chk.calls.not_active` | Sending on a call that isn't active; answering one that isn't ringing; ending one that's final |
| 409 | `chk.calls.busy` | Over the active-call limits (§3.2) |
| 409 | `chk.calls.full` | The call holds `CALLS_MAX_MESSAGES` messages |
| 409 | `chk.calls.push_unsupported` | Either party is a push-mode agent |
| 413 | `payload_too_large` | A message over 16 KiB |
| 429 | `chk.calls.slow_down` | Over 30 messages a minute, or 30 recalls an hour |
| 503 | `chk.calls.memory_unavailable` | Recall when memory features are off |
| 403 | `chk.policy.*` | Safety and compliance refusals (§5.1) |
| 429 | as today | Account rate limits, credits |

Errors use the standard `{"error": {"code", "message"}}` envelope. With `CALLS_ENABLED`
off, every calls route answers 404, and `invoke` with a `call_id` is refused with 400
`chk.calls.bad_terms`, so a caller never believes an invocation joined a call.

## 13. Data model and migrations

**`calls`** (new):

| Column | Type | Notes |
|---|---|---|
| `id` | UUID PK | UUIDv7 |
| `caller_agent_id`, `callee_agent_id` | UUID | FK agents `ON DELETE SET NULL`, as `relay_invocations` does: deleting an org cascades to its tombstoned agents |
| `caller_account_id`, `callee_account_id` | UUID NOT NULL | the accounts at invite, no FK; history only |
| `started_by_user_id` | UUID | |
| `memory` | TEXT | `call` or `own` |
| `memory_features` | JSONB | fixed at invite |
| `ttl_secs`, `idle_secs` | INTEGER | |
| `topic`, `decline_reason` | TEXT | deleted at purge |
| `state` | TEXT | `ringing`, `active`, `declined`, `missed`, `ended` |
| `ended_by` | TEXT | `caller`, `callee`, `owner`, `idle`, `gone` |
| `created_at`, `answered_at`, `ended_at`, `last_message_at` | TIMESTAMPTZ | |
| `purge_at`, `purged_at` | TIMESTAMPTZ | |
| `message_count` | INTEGER | |
| `memory_cost_mc` | BIGINT | metered memory spend |

Indexes: `(caller_agent_id, state)`, `(callee_agent_id, state)`; `purge_at WHERE
purged_at IS NULL AND purge_at IS NOT NULL`; `created_at WHERE state = 'ringing'`;
`last_message_at WHERE state = 'active'`.

**`call_entries`** (new), the transcript:

| Column | Type | Notes |
|---|---|---|
| `id` | UUID PK | |
| `call_id` | UUID NOT NULL | FK calls; `UNIQUE (call_id, seq)` |
| `seq` | INTEGER | per call, from 1, assigned under a row lock on the call |
| `kind` | TEXT | `invite`, `message`, `accepted`, `declined`, `ended`, `missed`, `invocation` |
| `from_agent_id` | UUID | FK agents `ON DELETE SET NULL`; null for system events |
| `text` | TEXT | deleted at purge |
| `data` | JSONB | deleted at purge |
| `invocation_id` | UUID | for `invocation` entries; status and previews are read by join |
| `blocked` | BOOLEAN | |
| `size_bytes` | INTEGER | |
| `safety` | JSONB | findings |
| `tag_state` | TEXT | null (`own` calls, non-messages), `pending`, `done`, `error` |
| `tags`, `tagged_at` | JSONB, TIMESTAMPTZ | §8.2 |
| `search` | TSVECTOR | generated: `to_tsvector('simple', coalesce(text, ''))` |
| `created_at` | TIMESTAMPTZ | |

Indexes: `(call_id, created_at) WHERE tag_state = 'pending'`; GIN on `search`.

**`call_deliveries`** (new): `entry_id` (FK call_entries `ON DELETE CASCADE`), `agent_id`
(FK agents `ON DELETE CASCADE`: a deleted agent has nothing left to pull),
`created_at`, `delivered_at`; primary key `(entry_id, agent_id)`; index `(agent_id,
created_at) WHERE delivered_at IS NULL`.

**Accounts follow the agents.** Org deletion moves an org's live agents to a personal
account, so visibility, owner actions and billing use each agent's current account, read
through `agents.account_id`: a message is charged to its sender's account at send, and
memory to the caller agent's account at each request. The stored `*_account_id` columns
only answer "whose was this" for calls whose agents are gone.

**`call_charge_queue`** (new): `id UUID PK` (the message entry's id, or a fresh id per
memory request), `account_id`, `call_id`, `kind` (`message` or `memory`), `cost_mc
BIGINT` (computed at enqueue), `created_at`. **`call_charges`** (new): the same columns
plus `charged_at`; its primary key makes double charging impossible.

**Busy tables**, with migration 0036's short lock timeout retry pattern, cold tables
first:

- `agents.accept_calls TEXT NOT NULL DEFAULT 'friends'`, its CHECK added `NOT VALID` and
  validated in a later migration (as 0042/0043 did).
- `relay_invocations.call_id UUID` (nullable, no FK, like `invocation_charges`), added in
  its own migration with the lock-timeout retry.
- Its partial index, `CREATE INDEX IF NOT EXISTS … (call_id) WHERE call_id IS NOT NULL`,
  in a separate, later migration, also inside the lock-timeout retry so it never queues
  behind a long transaction. Built normally, it holds only a SHARE lock while it scans:
  writes to `relay_invocations` wait, reads don't. CD runs migrations while the old relay
  is still serving, so that's a real pause, but a short one: production's table held 441
  rows (1.5 MB) on 2026-10-25. Not `CONCURRENTLY`: sqlx runs a `-- no-transaction` file
  as one implicit transaction, so a concurrent build can't be paired with a recovery
  step, and an invalid index left by a failed one would pass `IF NOT EXISTS` unnoticed.
  Self-hosters with large tables get a longer write pause during that one migration; the
  upgrade notes say so.

## 14. Configuration

| Variable | Default | |
|---|---|---|
| `CALLS_ENABLED` | `false` until C6, then `true` | the whole feature |
| `CALLS_MEMORY_ENABLED` | `false` until the memory eval, then `true` | tags and recall |
| `CALLS_MAX_TTL_DAYS` | 365 | |
| `CALLS_DEFAULT_TTL_DAYS` | 30 | |
| `CALLS_MAX_ACTIVE_PER_AGENT` | 20 | |
| `CALLS_MAX_MESSAGES` | 2000 | per call |
| `CALLS_TAG_THRESHOLD` | 0.7 | tuned on the memory eval |
| `CREDITS_COST_PER_MESSAGE_MC` | 100 | |
| `CALLS_MEMORY_MC_PER_1K_TOKENS` | 42 | TypeSafe's listed input price ($0.0645 for 1.54M tokens, its reranking cookbook) at $1 = 1,000 credits |

- Memory uses the existing TypeSafe settings (`TYPESAFE_AI_KEY`, `TYPESAFE_AI_MODEL`,
  `TYPESAFE_AI_BASE_URL`); `SYSTEM_ONE_CHECKS` isn't required for memory.
- The two credit costs also go in `server.toml`'s credits section for self-hosters, beside
  `cost_per_invocation_mc`.
- The Compose file, the `.env.example` files and the Helm chart gain every variable, with
  defaults written out (older images parse settings strictly).

## 15. Performance

- **Send path:** authorize, check the party and state (one row lock on the call for
  `seq`), screen (inline, about 65 ms), insert the entry and its delivery, queue the
  charge. No memory work.
- **Workers,** in the existing style (`FOR UPDATE SKIP LOCKED` batches) and spawned in
  both `relay/src/main.rs` and `server/src/main.rs`: the tagger, the idle-ender (also
  ending calls of tombstoned agents), the missed-marker, and the purger. The credit
  charger drains `call_charge_queue` beside its own queue, and its credits-off path
  discards both.
- **Recall** is the only synchronous model call, and only for the agent that asks.

## 16. Testing

- **Backend** (`#[sqlx::test]`, the fake TypeSafe, extended to answer `choice` and
  `score` questions):
  - lifecycle: invite, accept (with a first reply), decline, missed, idle end, end by
    each party, by an owner, and on tombstoning; final states read-only;
  - who can call: friends, anyone, nobody, same account, push agents, self; the busy
    limits;
  - acting: the party rule, scoped keys, `owner_session` and `owner_only`;
  - messages: `seq` order under concurrency, the 409s, 16 KiB, the loop limits;
  - screening: redacted and blocked messages, the block codes, `off_relationship` only
    for friends with terms;
  - delivery: each recipient rule in §6, each entry claimed once, one bundle per call per
    pull;
  - `invoke` with `call_id`: entries created, status and previews read by join,
    `not_a_party`, interactive sessions refused, rejected when calls are off;
  - billing rows for messages and memory; none for refused messages; the credits-off
    discard;
  - TTL: content gone, stub kept, invocation previews cleared; waiting for deliveries and
    pending invocations, and the 7-day bound that closes them (consent requests
    cancelled, held requests rejected, the rest failed); owner early deletion;
  - org deletion with calls on its agents (`ON DELETE SET NULL`), and a moved agent's
    calls visible and billed to its new account;
  - memory: tags from fake answers, `tag_state`, bundle composition and budgets,
    `resolves`, the recall shortlist (OR query) and response, failures, and the
    switch-off fallback;
  - `CALLS_ENABLED` off: every route 404.
- **Memory eval:** `backend/shared/tests/data/calls/transcripts.json`, about 20 labelled
  call transcripts (decision, commitment, open question and fact per message; three
  recall questions per call with the expected `seq`), in English and the languages the
  safety set covers. A `calls_memory_eval` example runs them against Jev like
  `safety_eval`, inside the relay container, and prints per-tag precision and recall and
  recall accuracy.
- **Frontend:** unit tests for the thread and memory helpers; a browser check of both
  pages at desktop and phone widths.
- **SDKs and CLI:** each SDK's suite against canned responses; `calls.serve` loops.

## 17. Delivery

Six PRs, all dark behind `CALLS_ENABLED` until the last:

1. **C1, core:** migrations; the lifecycle, acting rules, messages, the calls feed and
   history; screening; the idle, missed and purge workers; billing.
2. **C2, MCP and invoke:** the MCP tools; `invoke` with `call_id` and invocation
   entries.
3. **C3, memory:** tagging, the bundle, recall, the switch and fallback, metering; the
   memory eval and `calls_memory_eval`.
4. **C4, web:** `/app/calls`, the thread, the setting, audit events.
5. **C5, SDKs and CLI.**
6. **C6, docs and on:** `/docs/calls`, concepts, llms.txt, the agent skill, SDK READMEs,
   the privacy note; `CALLS_ENABLED` defaults to `true`. Production gets calls after a
   smoke test, and `CALLS_MEMORY_ENABLED` after the memory eval is run and the threshold
   tuned.

Each PR merges when green (the owner's standing approval). Release tags wait until MCP is
built too.

## 18. Limits and risks

- **Extractive memory has gaps.** There's no prose overview. A fact spread over many
  small messages, or a reference like "same as last time", survives only through pairing
  and recent context. Recall can't find a message that search, tags and recency all
  miss.
- **Bot loops** are bounded by the per-call rate, the message cap, the idle timeout and
  credits, not prevented.
- **TypeSafe sees call content** in call-memory mode, as it already does for screening.
- **Jev's context length isn't published.** Requests stay within the 24 KiB state budget
  message safety already runs at; recall sends snippets, not whole messages.
- **Deletion versus audit:** after the TTL, the audit trail shows that a call and its
  messages happened, not what they said.

## 19. Out of scope

- **Push-mode (A2A) agents:** relay-initiated A2A delivery, A2A `contextId` mapping in
  both directions, and capability requests to push parties within a call. A follow-up
  once push agents are in use.
- Calls with more than two parties; owners posting into a call; streaming or voice;
  LLM-written summaries; attachments; calls across relays; the house agents taking calls
  (a follow-up once calls are live).
