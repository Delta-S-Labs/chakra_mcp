# Calls C5: the SDKs and the CLI — implementation plan

Derived from `2026-10-25-calls-design.md` (§ numbers refer to it); C1–C4 (#445–#449)
shipped the relay and the web, dark. This is C5 of §17: calls in the four SDKs and the
CLI. The SDK versions are already bumped and unpublished (TypeScript and Python 0.5.0, Go
v0.3.0, Rust 0.3.0); nothing is released until the coordinated release.

**Conventions:** branch `feat/calls-c5`. Each SDK follows its own existing style (naming,
errors, tests with mocked HTTP), and its README gains a "Calls" section.

---

## The contract (identical in every SDK)

### Endpoints the SDKs call

| Method and path | Body or query | Returns |
|---|---|---|
| `POST /v1/calls` | `{from_agent_id, to_agent_id, memory?: "call" \| "own", ttl_secs?, idle_secs?, topic?, message?}` | `201` Call |
| `GET /v1/calls` | `?agent_id=&state=&limit=&cursor=` | `{calls: Call[], next_cursor?}` |
| `GET /v1/calls/{id}` | | Call |
| `POST /v1/calls/{id}/accept` | `{agent_id, message?}` | Call |
| `POST /v1/calls/{id}/decline` | `{agent_id, reason?}` | Call |
| `POST /v1/calls/{id}/end` | `{agent_id}` | Call |
| `POST /v1/calls/{id}/messages` | `{agent_id, text, data?}` | `201` Entry |
| `GET /v1/calls/{id}/messages` | `?after=&limit=&q=` | Entry[] |
| `GET /v1/calls/{id}/memory` | | Memory |
| `POST /v1/calls/{id}/memory/ask` | `{agent_id, question}` | Recall |
| `GET /v1/inbox/calls` | `?agent_id=&limit=` (limit ≤ 25) | FeedItem[] |

`POST /v1/invoke` gains an optional `call_id`. Owner routes (`owner-end`, `DELETE …/content`)
are web-only and not in the SDKs. Errors use the existing error type; codes include
`chk.calls.bad_terms`, `not_accepted`, `not_a_party`, `owner_session`, `not_active`,
`busy`, `full`, `push_unsupported`, `slow_down`, `memory_unavailable` and the
`chk.policy.*` refusals.

### Types

```text
Call {
  id, state: "ringing"|"active"|"declined"|"missed"|"ended",
  caller: CallParty, callee: CallParty,
  memory: "call"|"own", memory_features: {tags: bool, recall: bool},
  ttl_secs, idle_secs, topic?: string|null,
  created_at, answered_at?, ended_at?, ended_by?: "caller"|"callee"|"owner"|"idle"|"gone",
  purge_at?, purged_at?, message_count, last_message_at?
}
CallParty { agent_id?: string|null, account_id, display_name?, account_slug? }
CallEntry {
  seq, kind: "invite"|"message"|"accepted"|"declined"|"ended"|"missed"|"invocation",
  from_agent_id?, text?, data?, blocked: bool, safety?: SafetyFinding[],
  invocation?: {id, capability?, status?}, created_at
}
CallMemory { pinned: MemoryItem[], open: MemoryItem[], recent: MemoryItem[] }
MemoryItem { seq, from?, at, text?, context?, tags?: {decision, commitment, open_question,
             fact, weight}, invocation?: {id, capability, status, input?, output?} }
CallRecall { answered?: number|null, messages: [{seq, from?, text?, at, p}] }
CallFeedItem { call: Call, entry: CallEntry, memory?: CallMemory }
CallList { calls: Call[], next_cursor?: string }
```

`Agent` gains `accept_calls?: "friends"|"anyone"|"nobody"`, and the agent update request
takes it. `InvokeRequest` gains `call_id?`; `Invocation` gains `call_id?`.

### The `calls` namespace

`start`, `list`, `get`, `accept`, `decline`, `end`, `send`, `history`, `memory`, `ask`,
`pull` and `serve`, named in each language's style (`chakra.calls.start(...)` in
TypeScript and Python, `chakra.Calls().Start(ctx, ...)` in Go, `chakra.calls().start(...)`
in Rust). Python has it on both the sync and the async client.

### `calls.serve(agent_id, options)`

A loop for a pull agent, like `inbox.serve`:
1. Pull `GET /v1/inbox/calls` (batch up to 25). When it's empty, sleep the poll interval
   (default 2 s). Stop when the cancellation signal fires (an `AbortSignal`, a
   `threading.Event` or task cancellation, a `context.Context`, a cancellation future).
2. Handle each item in order:
   - **`invite`:** `on_invite(ctx)` returns accept (with an optional first reply) or
     decline (with an optional reason). The default, when no `on_invite` is given, is to
     accept.
   - **`message`:** `on_message(ctx)` gets `{call, entry, memory?}` and returns a reply,
     either text or `{text, data?}`, or nothing. A reply is sent as the agent with `send`;
     a `409 chk.calls.not_active` (the call ended meanwhile) is ignored.
   - **`accepted`, `declined`, `ended`, `missed`:** `on_event(ctx)` if given, else
     ignored.
3. A handler that throws or returns an error is logged (or reported through an error
   callback where the SDK has one) and the loop continues; it doesn't crash the server.

### Tests (each SDK, mocked HTTP)

- `start` sends the body and parses a Call;
- `list` passes the query and reads `next_cursor`;
- `send` and `history` round trip;
- `ask` parses a recall;
- an error with a `chk.calls.*` code surfaces it;
- `serve` accepts an invite by default, replies to a message through `on_message`,
  calls `on_event` for an end, survives a handler error, and stops when cancelled;
- `invoke` sends `call_id`.

## The CLI (`backend/cli`)

`chakramcp call` with these subcommands:
- `start <to-agent> --as <my-agent> [--memory call|own] [--ttl-days N] [--topic T]
  [--message M]`;
- `list [--as <agent>] [--state S]`;
- `show <call-id>` (the call and its transcript);
- `send <call-id> <text> --as <agent>`;
- `answer <call-id> accept|decline --as <agent> [--message M | --reason R]`;
- `ask <call-id> <question> --as <agent>`;
- `end <call-id> --as <agent>`.

Agents are named as in the other commands (an id, or a slug resolved among the user's
agents). `--json` prints the relay's JSON, as other commands do. Tests follow the CLI's
existing style.

## Verify

Each SDK's own checks (TypeScript `pnpm test` and `pnpm lint`; Python pytest, ruff and
mypy; Go vet and test; Rust clippy and test), the CLI's tests, and the workspace's
tests. Then the PR, merged when green.

## Open for C6's plan

`/docs/calls`, the concepts page, llms.txt, the agent skill, `CALLS_ENABLED` defaulting
to on, and the production switch-on after a smoke test.
