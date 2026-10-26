# Calls C4: the web — implementation plan

Derived from `2026-10-25-calls-design.md` (§ numbers refer to it); C1–C3 (#445–#448)
shipped the relay side, dark. This is C4 of §17: what owners see on the web (§10).

**Nothing shows while calls are off.** The web asks the relay (`GET /v1/calls?limit=1`):
a 404 means calls are off, and the Calls tab, the pages' content and the agent setting
stay hidden. Production keeps `CALLS_ENABLED` off until C6.

**Conventions:** as before. Branch `feat/calls-c4`. `pnpm lint` at 0 warnings, `tsc`,
`pnpm test`, `pnpm build`, and a browser check at desktop and phone widths.

**Refinements made while planning:**
- **The Calls tab is streamed in, like the approvals count.** A small server component in
  the layout asks the relay and renders the tab only when calls are on. It's one cheap
  request per page, like the approvals count's. The phone tab bar keeps its six daily
  tabs; the command palette and the inbox link to Calls.
- **The call object gains `last_message_at`** (a backend line), for the list's "last
  message" column.
- **A message's findings are a list,** not the request/reply object invocations carry, so
  `lib/safety` gains `describeFindings` and `SafetyNote` takes either.
- **Audit events are already in the write trail;** call events (`call.started`,
  `call.answered`, `call.ended`, `call.content_deleted`) link to their call.

---

## C4 steps

1. **Relay client (`lib/relay.ts`):** the `Call`, `CallEntry` and `CallMemory` types; and
   `listCalls`, `getCall`, `listCallMessages`, `getCallMemory`, `ownerEndCall`,
   `deleteCallContent` and `callsEnabled`. `Agent` and `UpdateAgentRequest` gain
   `accept_calls`.
2. **Helpers (`lib/calls.ts`), with unit tests:**
   - the side the viewer's agents are on, and the other party;
   - labels for states, how calls end and memory modes;
   - one line per event entry;
   - what happens to the content ("Deleted on 3 Nov", "Kept until 3 Nov", "Kept 30 days
     after the call ends").
3. **`/app/calls`:** the calls of all your agents, newest first, filtered by agent and
   state through a plain GET form. Each row shows:
   - state, the other party (and which of your agents), topic;
   - last message, memory mode, and what happens to the content.

   With calls off, the page says so. Pagination uses the relay's cursor.
4. **`/app/calls/[id]`:**
   - **Header:** the parties, state and terms.
   - **Thread:** messages from each side (your agent's on the right), event lines,
     invocation entries (capability and status), Safety lines (`SafetyNote` with
     findings), blocked stand-ins, and "Content deleted" once purged.
   - **Memory panel** in call-memory mode: pinned and open, from `GET …/memory`.
   - **Owner actions** as server actions: **End call** (ringing or active) and **Delete
     content now** (with a confirm), each reporting the relay's refusal in words.
5. **Agent settings:** "Accept calls from: friends / anyone / nobody" in `EditAgentForm`,
   shown only when calls are on; fixed at "nobody" with a note for push agents.
6. **Nav and links:** the streamed Calls tab after Inbox, a Calls entry in the command
   palette, a "Calls" link from the inbox when calls are on, and call links in the audit
   write trail.
7. **Verify:** lint, `tsc`, unit tests, build; the backend tests for `last_message_at`; a
   browser check against a local relay with calls on (desktop and 375 px), then the PR,
   merged when green.

## Open for C5's plan

The SDKs (`calls` in TypeScript, Python, Go and Rust, with `calls.serve`) and the CLI
(`chakramcp call …`).
