# Owner consent PR 3: verified human-in-the-loop — implementation plan

Derived from `2026-10-23-owner-consent-design.md` §5 (gate 2); PRs 1, 2a and 2b
(#433–#435) shipped gate 1. This PR changes behaviour for every `human_in_loop`
capability on pull-mode agents, in one go so it never half-works in production (§13):

- a reply an agent posts becomes a **draft**, until an approver confirms it on the web;
- a reply written in the web inbox by an approver is verified at once.

**Refinements made while planning:**
- **What the reply token covers.** Its `sha256` covers the RFC 8785 form of the reply
  as released, `{status, output?, error?}`, rather than the output alone, so a
  confirmed *failure* is bound too. It equals the draft's `draft_sha256`.
- **When it's minted.** The reply token is minted the first time the requester's side
  reads the released invocation, not at release. Release can happen in the app crate
  (the built-in page), which has no signing key, and minting on first read keeps all
  signing in the relay.

**Conventions** are as in the earlier plans.

---

1. **Shared decisions for replies (`shared::consent::decide`):**
   - **The approval view** gains `draft` (`{status, output?, error?}`) and
     `draft_sha256`, both only for those who may decide. For a reply, `input` is the
     original request's input.
   - **`lock_for_decision`** accepts `kind = 'reply'`, and requires the invocation to
     be `in_progress`.
   - **`approve` for a reply** takes `draft_sha256`, and returns a 409 "The draft
     changed, review it again" on a mismatch. It then:
     - marks the request `approved` with the decider;
     - releases the draft: the invocation's status, output, error and `elapsed_ms`;
     - records `consent.approve`, plus metrics for the request (`kind = "reply"`) and
       the invocation outcome;
     - sends the agent a `reply_confirmed` notice.
   - **`deny` for a reply:** the request becomes `denied` and the invocation `failed`
     with "The owner declined to answer: <note>". The agent gets a `reply_rejected`
     notice.
   - **Shared metrics** gain `record_kind(kind, outcome, n)` and
     `record_finished(status, elapsed_ms)`. The consent series are registered at 0 for
     `kind = "reply"` too.
2. **Storing drafts (`shared::consent::reply`, new):**
   - **`store_draft(db, inv, draft, wait_secs)`** upserts the invocation's single
     `kind = 'reply'` request. A new draft replaces a pending one, and `expires_at`
     resets. It returns the request id and expiry.
   - **`confirm_now(db, actor, inv, draft)`:** the approver-in-the-web-inbox path. In
     one transaction it stores the draft as `approved` by the actor, and releases it.
   - **The size limit:** the draft's serialized JSON may be at most 16 KiB, else
     `413 chk.result.too_large`.
3. **`submit_result` (`relay/src/consent/reply.rs`, new).** REST `report_result` and
   MCP `respond` both call it.
   - **Shared checks:** status (`succeeded` or `failed`), the invocation is
     `in_progress`, and the caller is a member of the target's account.
   - **Non-`human_in_loop` capabilities:** finish as today. REST keeps truncating the
     output at 16 KiB, and MCP keeps storing it whole. A `preview` parameter lets each
     surface keep its behaviour.
   - **`human_in_loop` capabilities:**
     - `confirmed_by_human` is ignored.
     - If the caller is an interactive web session (no API key, a JWT that isn't
       delegated) and `may_decide`, it calls `confirm_now` and returns the finished
       invocation.
     - Otherwise it calls `store_draft` and returns `202 {status:
       "awaiting_confirmation", consent_request_id, approve_url, expires_at}`.
   - **What it replaces:** the old 409 `chk.policy.requires_human_confirmation`
     gates in both handlers.
   - **MCP `respond`** returns the same object for a draft.
4. **Reading released replies:**
   - `InvocationDto` gains `human_confirmed: {consent_request_id, confirmed_at,
     token}`, from the approved reply request, in `list` and `fetch_one`.
   - `consent::deliver::reply_tokens(state, ids)` mints and stores missing tokens
     (`kind: "reply"`, `exp = iat + 30d`, `sha256 = draft_sha256`), as `deliver` does
     for request tokens.
5. **Expiry worker:** also expires pending reply requests. Their invocations fail
   with "Reply not confirmed in time", and the agent gets a `reply_expired` notice.
6. **Notices:** the `reply_*` kinds carry `{consent_request_id, invocation_id}`, and
   the notice claim expands them with the capability name and the outcome.
7. **Capability settings and `semantics`:**
   - REST `PATCH` accepts `semantics`.
   - The web capabilities panel adds "A person writes the reply" to the add form and to
     the Approvals editor.
   - CLI `capabilities update --semantics`.
8. **Web:**
   - **The approval page** for a reply reads "Confirm a reply?":
     - it shows the request's input, then the draft (status, output fields or the
       error). If the agent replaces the draft after the page loaded, confirming the
       old one returns "The draft changed, review it again";
     - Confirm sends `draft_sha256`;
     - Reject, with a note.
   - **The Approvals lists** label replies.
   - **The inbox:** answering a `human_in_loop` call shows "Confirmed and sent" for an
     approver, and "Saved as a draft for an approver to confirm" otherwise.
   - **The built-in self-hosted page** shows the same, with the hash in a hidden field.
9. **CLI:** `message reply` and `inbox respond` print "Waiting for confirmation:
   <approve_url>" on a 202.
10. **Tests:**
    - **Relay:**
      - an agent reply becomes a draft (REST and MCP);
      - the bare flag is ignored;
      - a replaced draft returns 409 on a stale hash;
      - approve releases the reply, with a token bound to it;
      - reject fails the invocation;
      - expiry fails it;
      - drafts over 16 KiB are refused;
      - a web-session approver's reply is verified at once;
      - a non-approver's web reply becomes a draft;
      - `reply_*` notices;
      - non-HITL capabilities are unchanged.
    - **App:** the built-in page confirms a draft and refuses a stale one.
    - **Frontend:** reply helpers.
11. **Verify in the browser** (local, managed mode):
    1. A HITL capability is called, and the agent posts a reply.
    2. The draft shows on the approval page.
    3. Confirm releases it with a token.
    4. A web-inbox reply by the owner goes straight through.
