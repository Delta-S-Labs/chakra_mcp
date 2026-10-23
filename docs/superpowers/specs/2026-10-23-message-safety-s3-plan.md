# Message safety S3: replies and notes — implementation plan

Derived from `2026-10-23-message-safety-design.md` (§ numbers refer to it). S1 (#440) and
S2 (#441) shipped the screener and request screening. This is S3 of §9: replies, notes,
and `safety` on invocations, all behind `SYSTEM_ONE_SAFETY`, which is off in production.

**With the switch off, every path runs today's code.**

**Conventions:** as in S2.

**Refinements made while planning:**
- **Notes go through one shared helper, not a screener argument.** The plan was to pass a
  screener into `decide::deny`. Instead, `system_one::note_or_refuse` screens a note and
  returns the text to store, or the refusal, and every note path calls it before writing:
  - friendship propose, accept, reject and counter (REST, and MCP's own
    `propose_friendship`);
  - grant create;
  - deny on the relay's approvals API;
  - deny on the app's built-in page.

  So `decide::deny` keeps its signature.
- **Owner-written web replies need no path of their own.** The web inbox posts to the
  relay's `/result` like any agent, so `submit_result` screens them, before `confirm_now`.
- **Push responses are screened inside the forwarder,** which receives a `ReplyScreen`
  (the checker and the capability context). Screening has to happen before the persist,
  and the forwarder is where the response is read.

---

## S3 steps

1. **Shared (`system_one/screen.rs`):**
   - **`ComplianceChecker::screen_note(text, about: NoteAbout)`** screens a note as
     `Channel::Note` at root `note`. `NoteAbout` carries the capability's name and
     description, for a grant purpose or a deny note, and the proposal message, for a
     friendship response.
   - **`pub async fn note_or_refuse(checker: Option<&ComplianceChecker>, text:
     Option<String>, about) -> Result<Option<String>, ApiError>`:**
     - unchanged when the checker is absent, safety is off, or there's no text;
     - a block → `ApiError::Policy { code, message }`;
     - over the cap → `ApiError::PayloadTooLarge("chk.safety.too_large")`;
     - otherwise the redacted text.
     - Findings are logged with the note's kind and recorded in metrics (channel
       `note`). Notes have no row, so nothing else stores them.
2. **Replies (`relay/src/consent/reply.rs`, `submit_result`):**
   - The query gains `c.name`, `c.description` and `c.output_schema`.
   - After authorization, with safety on, screen `{status, output?, error?}`
     (`Channel::Reply`, root `""`, schema `output_schema` at `/output`).
     - **Blocked:** store the report in `trust_snapshot.system_one.reply_safety`
       (jsonb merge), then `ApiError::Policy`. The invocation stays `in_progress`.
     - **Delivered:** the redacted output and error replace the caller's, and the
       report is stored the same way.
     - Then the existing flow runs (`confirm_now`, `store_draft`, or finish) on the
       screened values. A draft's `draft_sha256`, and so its reply token, covers the
       redacted reply.
   - **`Submitted`** gains `safety: Option<Safety>` (`reply` findings).
     - `draft_body()` includes it.
     - REST's finished response is the invocation DTO, which reads it from the
       snapshot (step 4).
     - MCP's finished JSON adds it.
3. **Push responses (`forwarder.rs`):**
   - `forward_push` gains `reply_screen: Option<ReplyScreen<'_>>`.
   - **With it:**
     - the body is read with a 1 MiB + 1 byte cap;
     - over the cap, the response is replaced by a JSON-RPC error with
       `data.code: "chk.safety.too_large"` (HTTP 502), and the call is recorded
       `failed`;
     - otherwise the body is screened (a JSON body as JSON at root `""`, anything else
       as text at `body`).
   - **Blocked:** the response becomes HTTP 403 with `-32009` and
     `data.code: "chk.policy.safety_blocked"`. The row is `failed` with
     `output_preview = {"__chakramcp_blocked__": true}`.
   - **Delivered:** the redacted body goes back to the caller. A JSON result object gains
     `metadata["chakramcp.safety"] = {reply: [...]}`.
   - **Either way:** the report goes in `trust_snapshot.system_one.reply_safety`, written
     by the same insert.
   - **Without it** (switch off): `resp.bytes()` exactly as today.
   - `handle_send_message` builds the `ReplyScreen` from its `ComplianceContext` when the
     switch is on.
4. **`safety` on invocations (`invoke.rs`):**
   - `InvocationDto` gains `safety: Option<Safety>` (omitted when empty), from
     `safety_from_snapshot(trust_snapshot, i_served, status)`:
     - `request`: `system_one.safety.findings`;
     - `reply`: `system_one.reply_safety.findings`. The caller sees it only once the
       reply is released (`succeeded` or `failed`), as it never sees a pending draft.
       The serving side always sees it.
   - The inbox query gains `i.trust_snapshot`. `list` and `fetch_one` already select it.
   - **MCP:**
     - `poll_invocation` returns the DTO's `safety` if it builds its own JSON (checked
       while building);
     - `list_invocations` bridges to `list`.
   - A2A `tasks/get` metadata gains `reply` findings once released.
5. **Notes:**
   - **`friendships.rs`:**
     - `propose`: `proposer_message`, about the proposal;
     - `accept`, `reject`: `response_message`, with the proposal message as context;
     - `counter`: both messages.
     - MCP's `propose_friendship` (its own SQL) gets `state` instead of `db`, and calls
       the same helper.
   - **`grants.rs` `create`:** `purpose`, about the capability.
   - **`approvals.rs` deny (relay):** the note, about the capability.
   - **`app/src/pages/approvals.rs` deny (built-in page):** the same, with the app's
     checker. A refusal re-renders the page with the reason, as other decision errors
     do.
6. **The app's client:**
   - `AppState` gains `system_one: Option<Arc<ComplianceChecker>>`;
   - `app/src/main.rs` builds it with `ComplianceChecker::from_env()`;
   - `server/src/main.rs` builds it once and gives the same `Arc` to the app and the
     relay (`with_compliance` gains an `Arc` form).
7. **Tests (fake TypeSafe, `#[sqlx::test]`):**
   - **Replies:**
     - REST and MCP: a reply redacted; the `safety.reply` in the response and the DTO;
     - blocked: 403 `chk.policy.safety_blocked`, the invocation still `in_progress`, the
       report stored;
     - a HITL draft stored redacted, its `draft_sha256` matching the redacted reply;
     - an approver's web reply redacted before `confirm_now`;
     - the caller not seeing `safety.reply` on a pending draft, and seeing it after
       confirmation;
   - **Push:** a response redacted with metadata; blocked → 403 and a `failed` row with
     the marker; over 1 MiB → too large;
   - **Notes:**
     - each friendship path and grant purpose redacted or refused;
     - the relay's deny note redacted;
     - the app's built-in page deny note refused, showing the reason;
   - **Off:** every path unchanged (the existing suites).
8. **Verify:** `cargo test --workspace`, clippy, and the sqlx cache.

## Open for S4's plan

The web "Safety" line and refused-note messages, the SDK types, and the docs
(`/docs/safety`, `docs/system-one-compliance.md`, llms.txt, the agent skill).
