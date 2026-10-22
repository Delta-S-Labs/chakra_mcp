# Owner consent PR 1: gate 1 backend, dark — implementation plan

Derived from `2026-10-23-owner-consent-design.md` (approved 2026-10-23); § numbers refer
to that spec. This is the plan for PR 1 of §13. PRs 2–4 get their own plans once this
one has merged, because they build on its code.

PR 1 changes nothing anyone can see. The publish and update APIs don't accept
`consent` yet, so no capability in production can require consent. Every new path runs
only in tests until PR 2.

**Conventions**
- **Ship flow:** branch from `feat/owner-consent` → PR → green CI → squash merge →
  check *that commit's* CD run (`gh run list --commit <sha>`). Commits end with the
  Co-Authored-By trailer, and PR bodies end with the Claude Code footer.
- **sqlx cache:** every new `sqlx::query!` needs `cargo sqlx prepare --workspace --
  --tests` against the local Postgres on `:5544`, with `DATABASE_URL` exported in the
  same shell as `git commit`. Commit `backend/.sqlx`.
- **Before pushing:** `cd backend && cargo test --workspace` and `cargo clippy
  --workspace --all-targets -- -D warnings`, and `cd sdks/rust && cargo test`.
- **Migrations are forever:** 0040–0043 are new; no applied migration is edited.
- **Invocation path:** capabilities without consent run exactly the code they run
  today. The new work is one branch on a column that's already loaded.

**Refinements made while planning** (the spec's intent, a simpler mechanism):
- **One shared enqueue function, not a `macro_rules!` splice (§4.1).** The trusted,
  public and A2A inserts write the same `relay_invocations` columns. So the
  consent-required case is a single function with one `sqlx::query!`, and all three
  paths call it. Their existing non-consent statements stay as they are.
- **Notices store only `{consent_request_id}`.** The notices endpoint (PR 2) joins
  the names, terms and URL at read time, so the invoke statement doesn't have to load
  display names.
- **`consent_terms` is accepted in PR 1** on REST, MCP and A2A invoke, because the
  shared enqueue function needs it and the tests drive it over HTTP. It's ignored
  unless the capability requires consent, which nothing in production can until PR 2.
  The CLI flags and docs stay in PR 2.

---

## PR-1 steps

1. **Migrations (§9).** Each busy table gets its own short file, and the scans go in a
   file of their own.
   - **`0040_consent_tables.sql`:** `consents`, then `consent_requests`, then
     `agent_notices`.
     - **Columns and checks:** as in §9. The `kind` and `status` CHECKs are inline,
       `decision_note` and `purpose` are each ≤ 500 characters, `max_requests > 0`
       when set, and `used` is between 0 and `max_requests`.
     - **Foreign keys:** the only one is `consent_requests.consent_id →
       consents(id)`, between two new tables. There are none to existing tables, not
       even `users`.
     - **Indexes:**
       - `consents (capability_id, grantee_agent_id) WHERE revoked_at IS NULL`,
         unique;
       - `consent_requests (status, expires_at)`;
       - `consent_requests (granter_agent_id, status)`;
       - `consent_requests (invocation_id, kind)`, unique;
       - `agent_notices (agent_id, created_at) WHERE delivered_at IS NULL`;
       - `agent_notices (created_at)`.
     - A header comment: owner consent PR 1, with a pointer to the spec.
   - **`0041_capability_consent.sql`:** inside the 0036 retry loop (100 ms
     `lock_timeout`, back off, at most 50 tries), lock `agent_capabilities` and add
     the four columns of §9.
     - **Constraints**, each added `NOT VALID`:
       - `consent_max_validity_secs IN (0, 3600, 86400, 604800, 2592000)`;
       - `consent_wait_secs BETWEEN 300 AND 604800`;
       - `cardinality(consent_approvers) <= 20`.
     - The header comment explains the locking, as in 0036.
   - **`0042_invocation_awaiting_consent.sql`:** the same loop on `relay_invocations`.
     One `ALTER TABLE` drops `relay_invocations_status_check` (from 0007) and adds it
     back with `'awaiting_consent'` included, `NOT VALID`.
   - **`0043_validate_consent_checks.sql`:** `VALIDATE CONSTRAINT` for the three
     checks from 0041 and the status check from 0042.
     - The comment says it takes only `SHARE UPDATE EXCLUSIVE`, so reads and writes
       continue during the scan.
2. **Shared interactive check (§7):**
   - Move `is_delegated_token` from `backend/app/src/auth.rs` to a new
     `backend/shared/src/sessions.rs`, as `pub async fn is_delegated_token(db, jti) ->
     Result<bool, ApiError>`. The app's `InteractiveUser` and `AdminUser` call it
     there.
   - **The relay's own `InteractiveUser`** goes in `backend/relay/src/auth.rs`. It
     extracts `AuthUser`, then returns 403 in either case:
     - `api_key_id.is_some()`;
     - `minted_jti` is `None` or delegated.
3. **413 error (`backend/shared/src/error.rs`):** add `ApiError::PayloadTooLarge
   { code: &'static str, message: String }`, which maps to 413 with that code. Used for
   `chk.request.too_large` now and for `chk.result.too_large` in PR 3.
4. **Consent module (`backend/relay/src/consent/`, new):**
   - **`terms.rs`:**
     - `ConsentTerms { purpose: Option<String>, valid_for: Validity, max_requests:
       Option<i32> }`, with `Validity::{Once, Secs(i64)}`, and serde for
       `"once" | "<n>h" | "<n>d"`.
     - **`cap(requested, max_validity_secs)`:** with a limit of 0, everything is
       `Once`.
     - **`within(approved, requested, max)`:** used when approving. It holds only if
       the approved validity ≤ the requested ≤ the limit, the approved request limit ≤
       the requested one (a missing limit counts as unlimited), and the purpose is
       unchanged.
     - **Validation:** a purpose ≤ 500 characters, `<n>` between 1 and 720 hours or 1
       and 30 days, and `max_requests` between 1 and 10,000.
     - **Unit tests** cover every rule.
   - **`enqueue.rs`:**
     `pub(crate) async fn enqueue_consent_required(db, NewInvocation, ConsentGate) ->
     Result<Enqueued, ApiError>`.
     - **`NewInvocation`** holds the columns the three paths share:
       - `id`, `grant_id`, `granter_agent_id`, `grantee_agent_id`, `capability_id`,
         `capability_name`;
       - `invoked_by_user_id`, `input_preview`, `api_key_id`, `minted_jti`,
         `trust_snapshot`, `charge_account_id`.
     - **`ConsentGate`** holds the capped `terms` and `wait_secs`.
     - **One statement** (§4.1):
       - `used` = the conditional `UPDATE consents … RETURNING id`;
       - insert the invocation with `status = CASE WHEN EXISTS (SELECT 1 FROM used)
         THEN 'pending' ELSE 'awaiting_consent' END`;
       - queue the charge `SELECT … FROM used`, so only covered requests are charged;
       - insert the consent request: `approved`, with `consent_id` and `decided_at =
         now()`, when covered; otherwise `pending` with `expires_at = now() +
         wait`;
       - insert a notice `SELECT … WHERE NOT EXISTS (SELECT 1 FROM used)`, of kind
         `consent_requested` with payload `{consent_request_id}`.
     - **The result** is `Enqueued::{Pending, Held { consent_request_id }}`.
     - **The size limit:** refuse an `input_preview` over `PREVIEW_BYTE_LIMIT`
       *before* this function runs, so the stored input is never a truncation marker
       (§3).
   - **`token.rs` (§6):**
     - `ConsentClaims { iss, jti, kind, inv, cap, granter, grantee, approver,
       consent?, sha256, iat, exp }`;
     - `mint(claims, &SigningKey)`, with header `{"alg":"EdDSA","kid","typ":
       "chakramcp-consent+jwt"}`, built like `jwt_mint::mint_jwt`;
     - `fn sha256_jcs(&Value) -> String`;
     - `verify(token, jwks, now)` for tests, which checks `typ`.
     - **Canonical JSON:** add one maintained RFC 8785 crate (`serde_jcs` or
       `serde_json_canonicalizer`, whichever is maintained at implementation time),
       pinned exactly. Test it against the RFC 8785 test vectors (Appendix B), and
       commit the vectors to `backend/relay/tests/data/jcs/`, so PR 4's SDKs share
       them.
   - **`signer.rs`:** `ConsentSigner { keystore: KeyStore, cached:
     RwLock<Option<(Arc<SigningKey>, Instant)>> }`, with `async fn key()`. It re-reads
     `current_active_key()` when the cache is older than 5 minutes, and falls back to
     `ensure_active_key()` when there is no key. `RelayState` gains `consent_signer:
     Arc<ConsentSigner>`.
   - **`approvers.rs`:**
     - `async fn may_decide(db, user_id, granter_account_id, approvers: &[Uuid]) ->
       bool`. One query decides: the user is a member of the account and either in
       `approvers`, or no listed approver is still a member and the user's role is
       `owner` or `admin` (§2).
     - `async fn effective_approvers(...)` returns the same set, for the approval view.
   - **`visibility.rs`:** `fn shown_input(i_served: bool, request_status:
     Option<&str>, input: Option<Value>) -> Option<Value>`. It returns
     `{"__chakramcp_held__": true}` when `i_served` is true and a gate-1 request
     exists that isn't `approved` (§4.3). Unit tests cover every combination.
5. **Invoke paths (§4.1).** Each path already loads its capability row; add
   `consent_required`, `consent_max_validity_secs` and `consent_wait_secs` to that
   `SELECT`.
   - **Trusted (`authorize_and_enqueue_trusted`):**
     - When the capability requires consent, check the size (413), cap the terms,
       then call `enqueue_consent_required` instead of the existing insert.
     - `TrustedOutcome` gains `Held { invocation_id, consent_request_id, signals }`.
     - `authorize_and_enqueue_trusted` takes `consent_terms: Option<&ConsentTerms>`.
   - **REST `invoke`:**
     - `InvokeRequest` gains `consent_terms: Option<ConsentTerms>`.
     - `InvokeResponse` gains `consent_request_id: Option<Uuid>`, skipped when None.
     - A held call returns `202 {status: "awaiting_consent", invocation_id,
       consent_request_id}`.
   - **Public (`invoke_public`):** the same branch. It charges `invoker.account_id`,
     and its quota check is unchanged: a held call counts as an attempt.
   - **MCP `invoke` (`mcp.rs` ~963):**
     - It takes `consent_terms`, and maps `Held` to `{invocation_id, status:
       "awaiting_consent", consent_request_id}`.
     - The tool schema documents the new argument.
   - **A2A:**
     - **Capability row:** `compliance_context` (`a2a.rs`) also selects the three
       consent columns.
     - **Terms:** read from `params.metadata["chakramcp.consent_terms"]`. Bad terms
       get a JSON-RPC `-32602` error with `data.code = "chk.consent.bad_terms"`.
     - **`park`** takes `Option<ConsentGate>`. With one, it calls the shared function,
       with `input_preview` = the raw `params` (already capped at 16 KB).
     - **The returned Task:** `state: "auth-required"` with the message "Waiting for
       the owner's approval" when held, otherwise `"submitted"`.
   - **Telemetry:** a held row isn't terminal, so `record_invocation_outcome` isn't
     called. `consent::metrics` counts it instead (step 12).
6. **Hidden held input (§4.3).** Each read gains a `LEFT JOIN consent_requests cr ON
   cr.invocation_id = i.id AND cr.kind = 'request'`, selects `cr.status AS
   "consent_status?"`, and passes `input_preview` through `shown_input`:
   - REST `list` and `fetch_one` (`invoke.rs` ~1268, ~1417);
   - MCP `list_invocations` (~785). It gains an `i_served` EXISTS on `ga.account_id`,
     copied from `list`.
   - MCP `poll_invocation` (~1008), the same way.
7. **Status allow-lists (§8):**
   - `awaiting_consent` joins the list filters in `invoke.rs` (~1282) and `mcp.rs`
     (~803), and the `list_invocations` tool schema (~317).
   - `inbox_bridge::get_task` maps `awaiting_consent` to `auth-required`, with the
     message "Waiting for the owner's approval".
   - The CLI, SDK and frontend lists wait for PR 2. The Rust SDK is step 15.
8. **Delivery and tokens (§4.5, §6):**
   - `InvocationDto` gains `consent: Option<ConsentDto { consent_request_id,
     consent_id, approved_at, terms, token }>`, skipped when None. MCP `pull_inbox`
     items gain the same `consent` object.
   - **`consent::deliver(state, &ids) -> HashMap<Uuid, ConsentDto>`** is called by
     REST `inbox` and MCP `pull_inbox` right after their claim:
     - it loads the gate-1 requests for the claimed ids;
     - for each request with no `token`, it mints one: `iat = now`, `exp = iat +
       24h`, `approver` = the deciding user or the covering consent's
       `approved_by_user_id`, and `sha256` over the delivered `input_preview`;
     - it stores the tokens with one `UPDATE … FROM unnest(...)`.

     Claims with no consent request skip all of this.
   - **`list` and `fetch_one`** return `consent` whenever the request is `approved`,
     with the stored `token`, or null if the invocation hasn't been delivered yet.
9. **Approval API (§7, §8).** New `backend/relay/src/handlers/approvals.rs`, with
   `InteractiveUser` on every route:
   - **`GET /v1/approvals?state=waiting|active|history`:**
     - **waiting:** pending requests the user may decide, newest first;
     - **active:** unrevoked, unexpired consents on agents in the user's accounts;
     - **history:** the user's accounts' decided requests in the last 90 days, at most
       200.
   - **`GET /v1/approvals/{id}`:** the full view for approvers:
     - the requesting agent, its account and verified flag;
     - the capability name and description, and the target agent;
     - the grant purpose and the terms;
     - the held input, and `expires_at`;
     - the effective approvers.

     A member who isn't an approver gets the same view without the input, plus
     `may_decide: false`. Anyone else gets 404.
   - **`POST /v1/approvals/{id}/approve {terms?}`** (§4.4), in one transaction:
     1. Lock the request (`FOR UPDATE`). It must be `pending` and unexpired;
        otherwise return 409 with its outcome.
     2. Check `may_decide` (403).
     3. Check the terms with `terms::within` (400).
     4. **Re-check authorization** (409 with the reason, after cancelling):
        - the grant is active and unexpired, with an accepted friendship; or the
          capability is `public_invoke` on a network-visible agent;
        - the capability still exists;
        - both agents and both accounts have `tombstoned_at IS NULL`.
     5. **Standing consent:** for multi-use terms, revoke any unrevoked consent for
        the pair, then insert one with `used = 1`.
     6. Mark the request `approved`, recording the decider, `approved_terms` and
        `consent_id`.
     7. Move the invocation from `awaiting_consent` to `pending`.
     8. Queue the charge.
     9. Record `consent.approve` in the audit log.

     **If the re-check fails,** the request becomes `cancelled` with the reason in
     `decision_note`, and the invocation becomes `rejected` with "No longer
     authorized: <reason>". No charge is queued. Record the rejection in telemetry and
     the audit log (`consent.cancel`).
   - **`POST /v1/approvals/{id}/deny {note?}`:** the request becomes `denied`, the
     invocation `rejected` with "The owner declined: <note>". Record `consent.deny`
     in the audit log.
   - **`POST /v1/consents/{id}/revoke`:** for a member of the granter's account who
     may decide. It sets `revoked_at` and `revoked_by_user_id`, and records
     `consent.revoke` in the audit log.
   - **`GET /v1/consent-requests/{id}`:** for any credential that is a member of
     either side. It returns status, kind, terms, `approved_terms`, `expires_at`,
     `decided_at` and the note, never the input (§8).
   - **Mounting:** routes go in `relay/src/lib.rs`, next to `/v1/invocations`.
10. **Expiry worker (§9). New `backend/relay/src/consent/expiry.rs`:**
    - **`spawn_expiry_worker(db)`** ticks every 60 s.
    - **Expire:** one statement expires up to 500 `pending` requests past
      `expires_at` (`FOR UPDATE SKIP LOCKED`). It rejects their `awaiting_consent`
      invocations with "Consent request expired", and records the rejections in
      telemetry.
    - **Prune:** a second statement deletes up to 1,000 notices older than 30 days,
      delivered or not.
    - **Wiring:** spawned from both `backend/relay/src/main.rs` and
      `backend/server/src/main.rs`, next to `spawn_usage_pruner`, and unconditionally
      (unlike the pruner).
    - **Testing:** an `expire_once(db, now)` function, so tests drive it without
      waiting.
11. **Cascades (§4.6, §9):**
    - **Grant revoke (`grants.rs` ~480):** the reject covers `status IN ('pending',
      'awaiting_consent')`, and a CTE cancels those invocations' pending consent
      requests. The rejection count for telemetry still comes from the invocation
      update.
    - **Capability delete:** in the capability delete handler, which the MCP bridge
      reuses, and in the same transaction:
      - pending requests for the capability become `cancelled`;
      - their `awaiting_consent` invocations become `rejected` ("Capability
        deleted");
      - its consents are revoked.
    - **Agent delete (tombstone):** the same for requests where the agent is the
      granter or the grantee ("Agent deleted").
    - **Reviews (`reviews.rs` ~243):** `i.status NOT IN ('rejected',
      'awaiting_consent')`.
    - **The policy comment (`policy/mod.rs` ~38):** replace the "future
      `ConsentRequired`" note with a pointer to `consent/` and the spec.
12. **Push-mode refusal (§11).** In `agents.rs` `update`, when the change is
    `CardChange::SetToPush` and the agent has a `consent_required` capability, return
    409: "This agent has capabilities that ask before delivering requests. Turn that
    off before giving it an endpoint." The publish-side refusal comes with the
    `consent` field in PR 2.
13. **Metrics (`relay/src/telemetry.rs`):**
    - **`CONSENT_REQUESTS_TOTAL{kind, outcome}`**, with `outcome` = `held`, `covered`,
      `approved`, `denied`, `expired` or `cancelled`. `describe()` registers each
      value at 0 for `kind = "request"`. PR 3 adds the `reply` values.
    - **`CONSENT_PENDING`**, a gauge the expiry worker sets on each tick.
    - **A test** renders `/metrics` and finds the series.
14. **Tests** (`#[sqlx::test(migrations = "../migrations")]`, using the
    `legacy_v01_contract_tests` fixtures and helpers in `invoke.rs`). They set
    `consent_required` directly in SQL, because publish can't yet. Everything in §12
    for gate 1:
    - **Holding:**
      - each path holds the request: REST trusted, REST public, MCP and A2A, which
        also checks `auth-required`;
      - inputs over 16 KiB are refused;
      - the inbox never claims `awaiting_consent`;
      - the charge is queued only at approval.
    - **Deciding:**
      - approve, narrow, deny and expire (with `expire_once`);
      - a standing consent covers a request, the request limit is reached, and two
        concurrent invokes race for the last use (`tokio::join!`, so exactly one is
        covered);
      - a revoked consent stops covering;
      - a non-approver gets 403, and so do an API key and a delegated token;
      - the approver list falls back to owners and admins when everyone listed has
        left;
      - approving after the grant is revoked cancels the request.
    - **Hidden input:** in REST list and get and MCP list and poll, including when one
      user owns both agents.
    - **Cascades:** grant revoke, capability delete and agent delete; reviews refuse a
      held request; switching to push mode is refused.
    - **Tokens:** a delivered token verifies against `/.well-known/jwks.json`. A
      changed input, a wrong `typ` and an expired token all fail.
15. **Rust SDK 0.3.0 (`sdks/rust`, §8, §11):**
    - **`InvocationStatus`** gains `AwaitingConsent`, a `#[serde(other)] Unknown`
      variant and `#[non_exhaustive]`. `is_terminal` is unchanged.
    - **`InvokeResponse`** gains `consent_request_id: Option<String>`.
    - **Release:** the version goes to `0.3.0`, with a CHANGELOG or README note and a
      test that parses `awaiting_consent` and some unknown status.
    - **Publishing:** after merge, tag `sdk-rust-v0.3.0`, but only with the user's
      go-ahead, because it publishes to crates.io. It must be out before PR 2 merges.
16. **This PR also commits** the spec and this plan.
17. **Verify locally** against `:5544`, with the server running:
    1. All four migrations apply on a copy of the current schema. Running
       `\d+ relay_invocations` shows the status check as validated.
    2. With `consent_required` set on a test capability in SQL, `POST /v1/invoke`
       returns 202 `awaiting_consent`, `/v1/inbox` returns nothing, and the target
       side's `GET /v1/invocations/{id}` shows the held marker.
    3. Approve with a web-session token: the inbox delivers it with a `consent.token`,
       which verifies against the JWKS. Approving with a `ck_` key returns 403.
    4. `/metrics` shows the consent series.

**After merge:**
- Check that commit's CD run, and that the relay logs show the expiry worker
  starting.
- In production, confirm `SELECT count(*) FROM consent_requests` is 0. Nothing can
  create one yet.

---

## Open for PR 2's plan

- **Self-hosted servers have no `/app` web UI.** They serve only the built-in sign-in
  pages, which is why `credits_url` is `None` there. PR 2 must choose between:
  - a minimal built-in approval page in the server, like the sign-in pages;
  - refusing `consent.required` when the server has no web UI.

  The first is better for self-hosters; the second is smaller.
- **`approve_url` is built at read time from `frontend_base_url`**, so the
  self-hosted answer only changes how it is built, not what is stored.
