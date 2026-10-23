# Message safety S2: requests — implementation plan

Derived from `2026-10-23-message-safety-design.md` (§ numbers refer to it); S1 (#440)
shipped `chakramcp_shared::safety`. This is S2 of §9: screening on every request path,
behind `SYSTEM_ONE_SAFETY`.

**With the switch off, every path runs exactly today's code.** Production has
`SYSTEM_ONE_CHECKS` on, so merging S2 changes nothing until the owner sets
`SYSTEM_ONE_SAFETY=true` on the VM, after the eval.

**Conventions:** as in S1.
- Branch `feat/message-safety-2`.
- `cargo test --workspace` and clippy before pushing.
- `cargo sqlx prepare --workspace -- --tests` for every new or changed query, with
  `DATABASE_URL` for `:5544` in the commit's shell.
- No token-shaped literals in tests (gitleaks).

**Refinements made while planning:**
- **The app wires the client in S3, not S2.** It only uses it in S3 (deny notes on the
  built-in page). S2 builds it in the relay and in `chakramcp-server`.
- **The eval example lives in `chakramcp-shared`**
  (`cargo run -p chakramcp-shared --example safety_eval`), beside the screener and the
  labelled set. The spec said the relay crate.
- **The fake TypeSafe server moves to the shared crate** with the client, behind a
  `test-support` feature, which the relay (and the app in S3) enable as a dev-dependency.
- **`consent_terms.purpose` is screened as part of the message.** The screened value is
  `{input, consent_terms: {purpose}}`, so a contact in the purpose is redacted like any
  other, and its path reads `consent_terms.purpose`.
- **S2 screens what the inbox delivers.**
  - REST and MCP screen `truncate_for_audit(input)`, the stored preview. An input over
    16 KiB is delivered as a marker, so there's nothing in it to screen.
  - A2A screens `params`, which is what `park()` stores and the push forwarder sends.

---

## S2 steps

1. **Move the client to `chakramcp_shared::system_one`:**
   - `relay/src/compliance/{mod,questions}.rs` become `shared/src/system_one/{mod,questions}.rs`.
   - Shared gains `reqwest = { workspace = true }`.
   - `crate::limits::enforce_flag` is replaced by a local `flag()` with the same values.
   - `relay/src/compliance.rs` becomes `pub use chakramcp_shared::system_one::*;`, so no
     relay call site changes.
   - `test_support` (the fake server and `checker()`) is
     `#[cfg(any(test, feature = "test-support"))] pub mod test_support`.
   - The relay's `[dev-dependencies]` gains
     `chakramcp-shared = { path = "../shared", features = ["test-support"] }`.
   - The existing compliance tests move with the code, unchanged.

2. **The switch:**
   - `ComplianceChecker::from_env` also reads `SYSTEM_ONE_SAFETY`, and `ComplianceChecker`
     gains `safety: bool`.
   - The switch on without checks or a key: `tracing::error!("SYSTEM_ONE_SAFETY is on but
     System One checks are off — safety screening DISABLED")`, and it stays off.
   - At boot it logs `System One safety screening enabled`.
   - `from_vars` tests cover the combinations.

3. **The questions (`system_one/questions.rs`, §4.2):**
   - `safety_questions(channel, plan_chunk_index, judged: &[CandidateView]) -> Map`:
     - `harm_violence`, `harm_crime`, `harm_harassment`, `harm_minors`, `harm_self`,
       `harm_malware` and `off_platform` on every chunk;
     - `contact_<id>` for each judged candidate, on the first chunk only;
     - `prompt_injection` reworded for replies (S3 uses it; written and unit-tested now).
   - Each question names `<channel>.fields` / `<channel>.decoded`, and the criteria treat
     quoted text as data.
   - With the switch on, today's request questions get a variant pointing at
     `request.fields` instead of `request.input` (`questions(subject, Shape::Fields)`).
     With it off, `Shape::Input` is today's text, byte for byte. A test pins the old
     wording.
   - **Thresholds:**
     - `BLOCK = 0.85`;
     - `NON_ENGLISH = 0.97`;
     - `HARM_MINORS = 0.5`;
     - `REDACT = 0.85`.
   - The `ids` lists: `BLOCKING_COMPLIANCE` (today's five), `BLOCKING_SAFETY` (`harm_*`),
     and `contact_*` / `off_platform`.

4. **`ComplianceChecker::screen` (`system_one/screen.rs`, §4.1–§4.3):**
   ```rust
   pub async fn screen(&self, channel: Channel, subject: &Subject<'_>, value: &Value,
                       root: &str, schema: Option<&Value>, schema_root: &str)
       -> Result<Screened, TooLarge>;
   pub struct Screened { pub value: Value, pub findings: Vec<Finding>,
                         pub block: Option<Block>, pub report: Value }
   pub struct Block { pub reason: String, pub code: &'static str }
   ```
   - **The code pass:**
     - `safety::scan`, then `safety::plan` with a `Context` from the subject;
     - each chunk's request is `{model, state: chunk.state, questions}`, with today's
       questions (in their `Fields` shape) plus the safety questions.
   - **Sending:**
     - every chunk at once (`tokio::task::JoinSet`; the client is cheap to clone);
     - a 4xx gets one retry with `safety::halve`, its remainder becoming `unjudged`
       findings;
     - an outage leaves that chunk's text `unjudged`.
   - **Combining:** each question's verdict is its highest probability across the chunks
     that answered.
   - **Policy (§4.3):**
     - the language comes from the scan;
     - block on any blocking question at or over its threshold. The reason names the
       highest one. `code` is `chk.policy.compliance_denied` for today's ids and
       `chk.policy.safety_blocked` for `harm_*`;
     - redact `contact_<i>` at or over the threshold;
     - `off_platform` at or over the threshold redacts `plan.unjudged_candidates`, or is
       flagged when there are none;
     - non-English answers in the band are flagged;
     - opaque spans and `unjudged` text are flagged.
   - **Every chunk failed:**
     - the report says `decision: "error"`;
     - every candidate is flagged with its kind (no `question` or `p`);
     - nothing is redacted or blocked.
   - **The value:** `safety::redact(value, &scan, &redacted_ids)`.
   - **The report:**
     ```
     {model, decision: allow|deny|error, language, thresholds,
      answers: {question: max_p}, chunks: n, failed_chunks: n,
      findings, elapsed_ms}
     ```
     It never holds a removed value.
   - **Metrics:** at the end, `findings_total` once per finding, and `screen_seconds`.
   - **Tests** use the fake server, which gains per-question probabilities and a per-call
     status sequence:
     - block;
     - redact one candidate of two;
     - redact past-16 candidates via `off_platform`;
     - flag `off_platform` with nothing to redact;
     - Spanish at 0.9 flagged, not blocked;
     - Spanish at 0.98 blocked;
     - `harm_minors` at 0.6 blocked in any language;
     - opaque flagged;
     - fail-open with findings;
     - a 4xx halving;
     - a 200 KiB message sending 4 requests, each body's `state` at most 24 KiB.

5. **Metrics (`shared/src/safety/metrics.rs`):**
   - `register()` sets every
     `chakramcp_safety_findings_total{channel, kind, action}` combination to 0 and
     describes `chakramcp_safety_screen_seconds{channel}`.
   - `record(channel, &findings, elapsed)`.
   - The relay's telemetry init calls `register()` (the app's, in S3).

6. **REST trusted path and MCP `invoke` (`handlers/invoke.rs`,
   `authorize_and_enqueue_trusted`):**
   - **Switch off:** today's `system_one_check`.
   - **Switch on:**
     - build `json!({"input": input_preview, "consent_terms": {"purpose": p}})` (the
       purpose only when present);
     - call `screen(Channel::Request, subject, …, root "", schema = the capability's
       `input_schema` (now selected in the grant query), schema_root "/input")`.
   - **`TooLarge`:** `ApiError::PayloadTooLarge { code: "chk.safety.too_large", … }` (no
     row, like other 4xx refusals before the row).
   - **Blocked:**
     - `reject()` records `input_preview = {"__chakramcp_blocked__": true}` and the
       report in `trust_snapshot.system_one.safety`;
     - `TrustedOutcome::Rejected` gains `code: Option<&'static str>` and
       `safety: Option<Safety>`.
   - **Delivered:**
     - the redacted `input` replaces `input_preview` in the insert (and in
       `enqueue_consent_required`);
     - the redacted purpose replaces `consent_terms.purpose` before `gate_for`;
     - the report goes in `trust_snapshot.system_one.safety`, beside today's
       `system_one` keys;
     - `TrustedOutcome::{Enqueued, Held}` gain `safety: Option<Safety>`.
   - **REST:** `InvokeResponse` gains
     `#[serde(skip_serializing_if = "Option::is_none")] code: Option<String>` and
     `safety: Option<Safety>` (omitted when empty).
   - **MCP:** `invoke`'s result gains `safety`. A `Rejected` with a code becomes a tool
     error whose JSON-RPC `data.code` is the code.

7. **REST public path (`invoke_public`):** the same as step 6, with no grant or friendship
   context. The capability query gains `input_schema`. The rejection and the 413 behave
   the same.

8. **A2A (`handlers/a2a.rs`, `inbox_bridge.rs`, `forwarder.rs`):**
   - **Switch on:** screen the envelope's `params` (`root "params"`, schema
     `input_schema`, schema_root `/params`… taken relative to `params`).
     - An unparseable body is screened as text with `safety::scan_body`.
   - **Blocked:** today's `-32009` response, with
     `data.code: "chk.policy.compliance_denied" | "chk.policy.safety_blocked"`. No row is
     written, as today.
   - **`TooLarge`:** `-32009` with `data.code: "chk.safety.too_large"`, HTTP 413.
   - **Delivered:**
     - the body is rebuilt with the redacted `params`;
     - when there are findings, `params.metadata["chakramcp.safety"] = {request: [...]}`
       is added;
     - the rebuilt bytes go to `park()` or `forward_push`.
   - **`park()`** gains `trust_snapshot: Option<Value>`, written in both its inserts
     (pending and consent).
   - **`forward_push` / `persist_invocation`** gain `trust_snapshot: Option<Value>`,
     written in the insert.
   - **The compliance report** (switch on or off) now goes in that `trust_snapshot`, so
     A2A gains the audit trail REST has.
   - **The parked task's response** carries `metadata["chakramcp.safety"]` when there are
     findings. `tasks/get` echoes it from the row's `trust_snapshot`.
   - **Tests:** pull and push, blocked and redacted, both `data.code`s, metadata on the
     forwarded body, `trust_snapshot` written, and switch off unchanged.

9. **Consent (§3):**
   - a held request's input and purpose are the redacted ones, so the approval page and
     the request token's `sha256` cover what's delivered;
   - test: a held request with a contact in its input shows `[contact removed]` in
     `GET /v1/approvals/{id}`, and its delivered token verifies against the redacted
     input.

10. **The eval (`shared/examples/safety_eval.rs`, §8):**
    - **Reads:**
      - `tests/data/safety/messages.json`;
      - `TYPESAFE_AI_KEY` (and optional `TYPESAFE_AI_MODEL` / `TYPESAFE_AI_BASE_URL`)
        from the environment. It never prints them.
    - **For each message,** runs `screen()` with a `Subject` from its capability, and maps
      the result to `deliver` / `redact` / `block` / `flag`:
      - `block` if blocked;
      - `redact` if anything was redacted;
      - `flag` if anything was flagged;
      - `deliver` otherwise.
    - **Prints:**
      - a table of pass and miss by group and by language, then each miss with its id,
        expected and got, and the top probabilities;
      - the median and p95 latency.
    - **Options:** `--only <id-prefix>` and `--threshold-report`. The latter prints, per
      question, the probabilities of the messages labelled for it, for tuning.

11. **Config:**
    - `infra/docker-compose.prod.yml` gains `SYSTEM_ONE_SAFETY: ${SYSTEM_ONE_SAFETY:-false}`;
    - `.env.example` and `infra/.env.example` list it with a comment;
    - the chart README notes `extraEnv`.
    - The docs (`docs/system-one-compliance.md` and others) come in S4.

12. **Tests (relay, `#[sqlx::test]`, fake TypeSafe):**
    - every path with the switch off: identical responses and rows (the existing tests,
      plus one asserting no `safety` field);
    - with the switch on:
      - REST trusted and public, and MCP: delivered with redaction (the row's
        `input_preview` and the `safety` in the response);
      - blocked with the marker row and the `code`;
      - a 1 MiB + 1 input refused 413, with no row;
      - the purpose redacted;
      - a held request redacted;
    - A2A as in step 8.

13. **Verify locally:**
    - Run the relay with the fake TypeSafe server as `TYPESAFE_AI_BASE_URL` (a tiny local
      binary, `shared/examples/fake_typesafe.rs`, behind `test-support`), with
      `SYSTEM_ONE_CHECKS=true` and `SYSTEM_ONE_SAFETY=true`.
    - Invoke a pull agent with `"mail me at bob@example.com"` and see the inbox receive
      `[contact removed]`.
    - The eval itself needs the real key, so the owner runs it.

## Open for S3's plan

Replies (REST `/result`, MCP `respond`, push responses with the capped read, owner web
replies), notes (friendships, grant purpose, deny notes including the app's built-in page,
and the app wiring the client), and `safety` on invocations.
