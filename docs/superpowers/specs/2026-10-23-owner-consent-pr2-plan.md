# Owner consent PR 2: gate 1 live — implementation plan

Derived from `2026-10-23-owner-consent-design.md` (approved 2026-10-23); § numbers refer
to that spec. PR 1 (#433) shipped the gate-1 backend switched off.

**Decision since the spec (2026-10-23).** Self-hosted servers get a **built-in approval
page**. They have no web app, only the server's built-in pages, so without one they
couldn't approve anything. That raises a question about where the approve and deny logic
lives:
- the built-in pages are in the app crate, which doesn't depend on the relay;
- so the logic moves to `chakramcp_shared::consent`, and both callers use it.

**Two PRs, so neither leaves a half-built flow live.** Gate 1 turns on when publish
accepts `consent`, so that field ships with the web pages.

- **PR 2a, still dark:** the shared decision code, the built-in approval page, notices,
  the new MCP tools, and CLI invoke flags and status lists.
- **PR 2b, live:** the web pages, and `consent` on REST, MCP and CLI publish and update.

**Conventions** are as in PR 1's plan. Frontend checks before pushing: `cd frontend &&
pnpm lint` (no warnings), `pnpm test` and `pnpm build`. Read the relevant guide in
`frontend/node_modules/next/dist/docs/` before writing Next.js code (Next 16.4, per
`frontend/AGENTS.md`).

---

## PR 2a: shared decisions, built-in page, notices, MCP and CLI (dark)

1. **Shared decision code (`backend/shared/src/consent/`, new).** These move here from
   the relay:
   - **`terms.rs`:** moved as it is.
   - **`approvers.rs`:** `may_decide`, `effective_approvers`.
   - **`decide.rs`** holds what is in `relay/src/handlers/approvals.rs` today:
     - `load_view(db, user_id, id) -> Option<ApprovalView>`, which includes `input`
       only for those who may decide;
     - `approve(db, user_id, id, Narrowing) -> Result<Approved, DecideError>`;
     - `deny(db, user_id, id, note) -> Result<Denied, DecideError>`;
     - `revoke_consent(db, user_id, id)`;
     - `still_authorized` and its unit test.
   - **`DecideError`:**
     - `NotFound` (404), `Forbidden` (403);
     - `AlreadyDecided(status)` and `Expired` (409);
     - `BadTerms(msg)` (400);
     - `NoLongerAuthorized(reason)` (409, after the request is cancelled);
     - `Db`.

     `impl From<DecideError> for ApiError`.
   - **Side effects** are recorded in shared, so both callers get the same ones:
     - **metrics:** the consent outcomes, and rejected invocations through
       `names::INVOCATIONS_TOTAL` with `mode=pull`;
     - **audit:** one `audit_events` insert, taking the actor user, account,
       `api_key_id` and `minted_jti` (both `None` for a web or page session).
   - **The relay** keeps the HTTP layer: `InteractiveUser`, JSON shapes, list queries.
     Its handlers call `shared::consent::decide`, and `relay::consent::{terms,
     approvers}` become re-exports. Tokens, enqueueing and delivery stay in the relay.
   - **Tests:** the relay's approval tests keep passing unchanged, which shows the move
     changed nothing.
2. **Built-in approval page (self-hosted; `backend/app/src/pages/approvals.rs`, new).**
   - **Routes:** `GET /app/approvals/{id}` and `POST /app/approvals/{id}`, mounted in
     `pages::router()`. They exist only with `HOSTING_MODE=self_hosted`, beside
     `/app/pair`.
   - **Signing in:** without a page session, show the sign-in form (the
     `pair.rs` / `SigninPage` flow), then come back.
   - **The page:** plain HTML (`templates/pages/approval.html`), laid out as §7 describes:
     - who's asking (agent, account, verified);
     - the capability and its description, and your agent;
     - why (the requester's purpose and the grant's purpose);
     - what they're sending: the input, as a definition list of top-level fields, with
       nested values as JSON;
     - for how long and how often: `<select>`s offering only values within the request
       and the publisher's limit;
     - when it expires.

     Buttons: Approve; Deny, with an optional note.
   - **The other states:**
     - non-approvers see who can approve;
     - decided, expired and cancelled requests show their outcome.
   - **Submitting:** checks CSRF and `Origin` as the other forms do, and calls
     `shared::consent::decide`.
   - **Tests** in `pages/tests.rs`:
     - signed out → sign-in;
     - approve with narrowing → the invocation is `pending`;
     - deny with a note;
     - a non-approver gets no buttons and a 403 on POST;
     - a stale CSRF token → "This form expired";
     - not mounted when managed.
3. **Notices (§4.2, `backend/relay/src/consent/notices.rs`, new).** `claim(state,
   user, agent_id, limit)` has the inbox's membership and scope checks, and marks the
   agent online.
   - **The claim:** `UPDATE agent_notices SET delivered_at = now() WHERE id IN (SELECT …
     WHERE agent_id = $1 AND delivered_at IS NULL ORDER BY created_at LIMIT $2 FOR
     UPDATE SKIP LOCKED) RETURNING …`.
   - **Expanding each `consent_requested` notice:** join the consent request, the
     requesting agent and account (with the verified flag) and the capability name. Add
     `terms`, `expires_at` and `approve_url` = `{frontend_base_url}/app/approvals/{id}`.
     That URL also works on self-hosted servers, where the frontend URL is the server
     itself.
   - **Never the input.**
   - **Routes and tools:** `GET /v1/inbox/notices?agent_id=&limit=` (default 25, at most
     100), and MCP **`pull_notices`** with the same arguments.
4. **MCP `consent_status`:** the logic of `GET /v1/consent-requests/{id}`, moved into a
   function both use.
5. **CLI (`backend/cli/src/commands/`):**
   - **`invoke`:**
     - new flags: `--consent <once|Nh|Nd>`, `--consent-max N`, `--purpose "…"`;
     - a held call prints "Waiting for the owner's approval (request <id>)";
     - `--wait` keeps polling through `awaiting_consent`.
   - **`invocations --status`** accepts `awaiting_consent`.
   - **`inbox notices --agent <id>`** prints each notice's requester, capability, terms,
     link and expiry.

## PR 2b: web pages and publish (live)

6. **Publish and update accept `consent` (§3).**
   - **The field:** `consent: {required?, approvers?, max_validity?, wait?}` on REST
     `POST`/`PATCH /v1/agents/{id}/capabilities` and MCP `publish_capability`.
     - `approvers`: user ids or member emails;
     - `max_validity`: `once`, `1h`, `1d`, `7d` or `30d`;
     - `wait`: `<n>m`, `<n>h` or `<n>d`, between 5 minutes and 7 days.
   - **Validation, with 400s:**
     - every approver must be a member of the agent's account (emails resolve to user
       ids);
     - the values must be allowed ones;
     - `required: true` on a push-mode agent is a 409 (§11).
   - **`CapabilityDto`** gains `semantics` and `consent: {required, approvers,
     max_validity, wait}`.
   - **Turning consent off** leaves already-held requests to be decided or to expire.
   - **MCP:** `publish_capability`'s own `INSERT` gets the same fields, sharing one
     validation function with REST.
7. **CLI `capabilities add|update`:** `--ask-first`, `--no-ask-first`, `--approver
   <email>` (repeatable), `--max-consent <once|1h|1d|7d|30d>` and `--consent-wait <Nm|Nh|Nd>`.
8. **Frontend client (`src/lib/relay.ts`):**
   - `InvocationStatus` adds `awaiting_consent`;
   - `Capability` gains `semantics` and `consent`;
   - the requests gain `consent`;
   - approval types, and `listApprovals`, `getApproval`, `approve`, `deny`,
     `revokeConsent`.
9. **`src/lib/consent.ts` (new, unit tested with `pnpm test`):**
    - the narrowing options (never past the request or the publisher's limit);
    - how terms are described ("Once", "For 7 days, up to 5 requests");
    - time remaining;
    - the held-input marker check.
10. **`/app/approvals/[id]`:** the consent screen, built for phones (§7), with the same
    content and states as the built-in page. Approving and denying are server actions.
    A 401 redirects to sign-in and back (`redirectIfUnauthorized`).
11. **`/app/approvals`:**
    - three tabs: Waiting, Active consents (with Revoke) and History;
    - the nav gets an Approvals tab with the waiting count (the layout fetches it), on
      `AppNav` and `BottomTabBar`;
    - the dashboard shows a banner when anything is waiting.
12. **Capability settings (`agents/[id]/CapabilitiesPanel.tsx`):** an Approvals section.
    - **"Ask before delivering requests"**, with the longest consent, how long to wait,
      and the approvers: org members to pick from, or "Owners and admins".
    - **On a push-mode agent** it's disabled, with the reason.
    - **"A person writes the reply"** comes in PR 3.
13. **Invocation views (inbox and audit):**
    - an `awaiting_consent` badge;
    - held input shown as "Held for the owner's approval".
14. **Tests:**
    - **Relay:** publish validation (approvers, values, push refusal), the DTO round
      trip, and an MCP publish with `consent`.
    - **Frontend:** unit tests for `consent.ts`.
15. **Verify locally in the browser at phone width** against a local relay and app:
    1. Publish an ask-first capability in settings.
    2. Invoke it from a second agent.
    3. `inbox notices` shows the link.
    4. Approve it on `/app/approvals/{id}`.
    5. The inbox delivers it with a token, and the Approvals tabs show it.

    For the self-hosted path, the same with `HOSTING_MODE=self_hosted` on the built-in
    page.

**After 2b merges:** check both CD runs. In production, publish an ask-first capability
on a test agent, run the flow once, then delete the capability.
