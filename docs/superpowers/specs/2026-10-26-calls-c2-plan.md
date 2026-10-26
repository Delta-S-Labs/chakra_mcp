# Calls C2: MCP and capabilities in a call — implementation plan

Derived from `2026-10-25-calls-design.md` (§ numbers refer to it); C1 (#445) shipped the
core, dark. This is C2 of §17: the MCP tools, and `invoke` with `call_id` (§7). Still dark
behind `CALLS_ENABLED`.

**With the switch off, nothing changes:** the tools aren't listed and `invoke` refuses a
`call_id` (400 `chk.calls.bad_terms`), so nobody believes an invocation joined a call.

**Conventions:** as in C1. Branch `feat/calls-c2`.

**Refinements made while planning:**
- **`accept_calls` on agent cards waits for push agents.** The published card and
  discovery search follow A2A's card schema, which has no slot for it, and only push
  agents are reached through cards. `/v1/agents` and `/v1/network/agents` carry it (C1).
- **The call is checked where the invocation's two agents are known:** after the grant
  (trusted path) or the capability (public path) is resolved, before usage limits,
  compliance or screening spend anything.
- **The transcript entry is written after the invocation row, best effort.** The row
  itself carries `call_id` in the same statement that creates it, so the purge always
  finds it; if appending the `invocation` entry then fails, the relay logs it and the
  invocation still stands. Making the entry atomic would mean a transaction around every
  invoke path for a history line.
- **Rejected invocations join the call too.** A request refused by compliance or safety
  inside a call gets `call_id` and an entry, so the thread shows it was tried and
  refused (its status says so).

---

## C2 steps

1. **`calls::link`:**
   - `check(state, user, call_id, grantee, granter)`:
     - calls off: 400 `chk.calls.bad_terms`;
     - the call must exist and be readable (404), and be `active` (409
       `chk.calls.not_active`);
     - the grantee must be a party, acting under §4's rule (so a web session gets
       `chk.calls.owner_session`), and the granter must be the other party; otherwise
       403 `chk.calls.not_a_party`.
   - `note(db, call_id, invocation_id, from)` appends an `invocation` entry (no
     deliveries: the invocation reaches its agent through the inbox), logging any
     failure.
2. **Threading `call_id`:**
   - `InvokeRequest` and MCP `invoke` gain `call_id`.
   - `authorize_and_enqueue_trusted`, `invoke_public` and `record_terminal` take
     `call: Option<Uuid>`; `consent::enqueue::enqueue_consent_required` takes it through
     its arguments.
   - Every insert on those paths writes `call_id`, and each calls `link::note` after
     its insert.
3. **Reading invocations:**
   - `InvocationDto` gains `call_id` (omitted when null), on `GET /v1/invocations`,
     `GET /v1/invocations/{id}` and `/v1/inbox`.
   - MCP `poll_invocation`, `list_invocations` and `pull_inbox` carry it too.
   - Transcript entries of kind `invocation` gain `invocation: {id, capability, status}`,
     read by join in history.
4. **MCP tools,** listed only when calls are on (`tools_list_result` takes the switch):
   `start_call`, `list_calls`, `get_call`, `answer_call` (`answer: accept | decline`),
   `send_message`, `pull_calls`, `call_history` and `end_call`. Each deserializes its
   arguments into C1's request types and calls the same functions; errors map through
   `api_err_to_rpc`, so `structuredContent.code` carries `chk.calls.*`.
5. **Tests** (`#[sqlx::test]`):
   - `invoke` with `call_id` on the trusted path: the row's `call_id`, the entry and
     its joined status, `call_id` on the invocation DTO and in the inbox;
   - refusals: a grantee not on the call, a capability of a third agent, an ended call,
     a web session, calls off;
   - a request refused by compliance inside a call still joins it;
   - MCP: a call from `start_call` to `end_call` over `/mcp` with API keys; the tools
     hidden and unknown with calls off.
6. **Verify:** `cargo test --workspace`, clippy, the sqlx cache; PR, merged when green.

## Open for C3's plan

Memory: Jev tagging, the bundle on the feed and `GET …/memory`, recall
(`POST …/memory/ask`, MCP `call_memory` and `ask_call_memory`), `CALLS_MEMORY_ENABLED`, the
fake TypeSafe's choice and score answers, metering, and the memory eval.
