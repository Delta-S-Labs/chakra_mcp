# Owner consent and verified human-in-the-loop

Status: approved design, 2026-10-23. First of four projects from the owner's notes
(Notion, "chakramcp", 2026-10-12). Order agreed: **consent → safety → calls → MCP**:

1. **Consent** (this spec). An agent's owner verifiably approves requests before
   their agent sees them, and verifiably confirms human-written replies.
2. **Safety.** Jev (System One) checks extend from invocation inputs to outputs and
   messages, adding off-platform contact detection, multi-language and encoded text.
3. **Calls.** Two-way sessions between agents, with a chosen memory mode (a shared,
   compacted call memory, or each agent's own), a TTL on call data, and the audited
   history grouped by call.
4. **MCP.** MCP as the front door for people and services: directory search,
   public-capability invoke and calls.

Calls build consent and safety in from the start; MCP exposes all three.

## 1. Why

Two gaps, found by reading the code (2026-10-23):

- **Nothing makes an agent ask its owner before acting.** For "book my calendar" or
  "pay this invoice", the owner wants to approve each request, or a bounded set of
  requests, before their agent acts on it. Today the agent sees the request the
  moment the relay queues it.
- **`human_in_loop` is self-asserted.** A capability with
  `semantics = 'human_in_loop'` only requires the reply to carry
  `confirmed_by_human: true` (`report_result`, `invoke.rs:1220`). Any agent can set
  it: the CLI's `message reply` and `inbox respond` always do, and MCP `respond` takes
  it as an argument. The flag isn't stored. The web inbox can't answer such calls at
  all: it never sends the flag, so it gets a 409.

Both are solved by the same mechanism: **a signed-in approver confirms, on the web,
exactly what they are agreeing to.**

## 2. Concepts

| | What the approver confirms | When | Capability setting |
|---|---|---|---|
| **Gate 1: request consent** | "Deliver this request to my agent." | Before the agent sees the request | `consent.required` |
| **Gate 2: verified reply** | "This reply is mine / I approve it." | Before the requester sees the reply | `semantics = 'human_in_loop'` |

The two are independent: a capability can use either, both or neither.

- **Approver.** A user allowed to decide for a capability: the publisher's list, or,
  when the list is empty, the agent account's owners and admins. Worked out when the
  decision is made.
- **Consent request.** One record per held request (gate 1) or per draft reply
  (gate 2). It moves from `pending` to `approved`, `denied`, `expired` or
  `cancelled`.
- **Consent.** Standing approval for a requesting agent and a capability, within a
  validity window and an optional request limit. It's created when an approver
  approves multi-use terms.
- **Consent terms.** What the requester proposes: purpose, validity (`once` or N
  days) and an optional request limit. The model is India's Account Aggregator: the
  requester proposes terms, and the approver accepts them, narrows them or refuses.
  They can't widen them.
- **Consent token.** A signed JWT stating what was approved. It's bound to the exact
  input (or released output) by its SHA-256.

## 3. Capability settings

Set when publishing or updating a capability (REST, MCP, CLI, web):

| Field | Meaning | Default and limits |
|---|---|---|
| `consent.required` | Gate 1 on | `false` |
| `consent.approvers` | User ids allowed to decide | Empty means owners and admins. Each must be a member of the agent's account when set; members who leave stop counting. |
| `consent.max_validity` | Longest consent an approver may grant | `0` (each request on its own). One of `0`, `1h`, `1d`, `7d`, `30d`. |
| `consent.wait` | How long a request or draft waits for a decision | `24h`. From `5m` to `7d`. |

`semantics` keeps its two values, `autonomous` and `human_in_loop`. What changes is
that `human_in_loop` is now verified through gate 2 (§5).

## 4. Gate 1: consent before delivery

1. **The requester invokes,** over REST `POST /v1/invoke`, MCP `invoke`, A2A
   `SendMessage` or an SDK. It may include
   `consent_terms: {purpose?, valid_for: "once" | "<n>d", max_requests?}`. The
   relay first runs the existing checks: the grant or public path, rate and credit
   limits, and Jev on the input. Terms beyond the publisher's `max_validity` are
   capped. With no terms, the request is `once`.
2. **An active consent covers it.** The consent is for the same requesting agent and
   capability, is in date, isn't revoked, and has uses left. Its use count is
   decremented under a row lock. The invocation is created `pending`, with an
   `approved` consent request linked to the consent, and is delivered and charged
   as usual.
3. **Otherwise the request is held.** The invocation is created with status
   `awaiting_consent`, with no credit charge queued. A consent request is created
   with `kind = 'request'`, the capped terms and `expires_at = now + wait`. An
   `agent_notices` row of kind `consent_requested` is created for the target agent.
   The requester gets `202 {status: "awaiting_consent", invocation_id,
   consent_request_id}`. The approval link is never returned to the requester.
4. **The owner's agent pulls its notices** (`GET /v1/inbox/notices`). Each carries:
   - the consent request id;
   - the requesting agent and account (display names and ids, and verified or not);
   - the capability name;
   - the terms;
   - `approve_url` (`{frontend}/app/approvals/{id}`) and `expires_at`.

   It never carries the request's input. The agent passes the link to its human.
5. **An approver decides,** in an interactive web session (§7).
   - **Approve,** as proposed or narrowed. In one transaction:
     - the consent request becomes `approved` (decided by, at, and the final terms);
     - multi-use terms create a `consents` row, with this request counting as its
       first use;
     - the invocation becomes `pending`, so the inbox claims it as usual;
     - the credit charge is queued;
     - the consent token is minted and stored on the consent request.
   - **Deny,** with an optional note of 500 characters or fewer. The consent request
     becomes `denied`, and the invocation becomes `rejected` with
     `error_message = "The owner declined: <note>"`. No charge.
6. **Expiry.** The worker (§9) marks the consent request `expired` and the
   invocation `rejected` ("Consent request expired"). No charge.
7. **Delivery.** `InvocationDto.consent = {consent_request_id, consent_id?,
   approved_at, terms, token}` appears on inbox pulls and on reads by either side.

The inbox claim query only selects `status = 'pending'`, so held requests are never
claimed, with no change to the inbox.

## 5. Gate 2: verified replies (`human_in_loop`)

1. **An agent posts a reply** for a `human_in_loop` capability, with any
   non-interactive credential: an API key, an OAuth or device token, MCP, or the
   CLI. `POST /v1/invocations/{id}/result` stores the draft (status and
   output/error) on a new consent request with `kind = 'reply'`. The invocation
   stays `in_progress`. The response is
   `202 {status: "awaiting_confirmation", consent_request_id, approve_url,
   expires_at}`. `confirmed_by_human` is ignored.
2. **An approver opens the link** and sees the original request and the draft.
   - **Approve:** the draft becomes the result, the invocation is `succeeded` or
     `failed` as drafted, and `human_confirmed = {consent_request_id, confirmed_at,
     token}` is set. The token binds the SHA-256 of the released output.
   - **Reject,** with an optional note: the invocation is `failed` ("The owner
     declined to answer: <note>"). The agent gets a `reply_rejected` notice.
3. **A reply typed in the web inbox** by a signed-in approver is verified at once.
   It's released directly, with a consent request recorded as `approved` by that
   user.
4. **A second draft before a decision** replaces the first. The approval page
   always shows the latest draft and says it was replaced.
5. **Expiry.** The invocation fails with "Reply not confirmed in time". The agent
   gets a `reply_expired` notice.

The agent learns the outcome through notices (`reply_confirmed`, `reply_rejected`,
`reply_expired`).

## 6. Consent token

The token is an EdDSA JWT signed with the relay's existing signing key
(`relay_signing_keys`). Its `kid` is published at `/.well-known/jwks.json`.

| Claim | Value |
|---|---|
| `iss` | Relay public URL |
| `typ` (header) | `chakramcp-consent+jwt` |
| `jti` | Consent request id |
| `kind` | `request` or `reply` |
| `inv` | Invocation id |
| `cap` | Capability id |
| `granter` / `grantee` | Target and requesting agent ids |
| `approver` | Approving user's id |
| `consent` | Consent id, when a standing consent covered it |
| `sha256` | Hex SHA-256 of the canonical JSON input (`request`) or released output (`reply`) |
| `iat`, `exp` | Issue time, and expiry. For `request`, the consent's `valid_until`, or 24 hours after `iat` for a one-off; for `reply`, 30 days after `iat`. |

Canonical JSON: keys sorted and no insignificant whitespace, the same everywhere a
hash is computed. The token grants nothing in the relay. Owners' own systems can
check it offline to confirm this exact request was approved.

## 7. Web

- **`/app/approvals/{id}`** is the link agents pass on. It's built for phones and
  laid out like an Account Aggregator consent screen:
  - who's asking (agent, account, verified badge);
  - the capability (name, description, your agent);
  - why (the requester's purpose and the grant's purpose);
  - what they're sending (the input, rendered as fields, or the draft reply for
    gate 2);
  - for how long and how often;
  - when the request expires.

  The approver can approve as asked or narrow the terms (options never exceed the
  request or the publisher's limit), or deny with a note. Non-approvers see who can
  approve. Decided, expired and revoked requests show their outcome.
- **`/app/approvals`** has three lists:
  - **Waiting:** what you can decide, with time left.
  - **Active consents:** requester, capability, valid until, uses n/M, with Revoke.
  - **History:** the last 90 days.

  The nav shows a count, and the dashboard shows a banner when anything is waiting.
- **Capability settings,** in the agent's Capabilities panel:
  - "Ask before delivering requests", with approvers, longest consent and wait;
  - "A person writes the reply" (`human_in_loop`).
- **The web inbox** answers `human_in_loop` calls (verified at once, §5.3). It shows
  `consent` and `human_confirmed` on invocations.

**Interactive sessions only.** Decisions go through the app API with
`InteractiveUser` (`backend/app/src/auth.rs`), which refuses API keys and delegated
(OAuth/device) tokens. An agent holding its owner's API key can't approve its own
requests.

## 8. API, MCP, SDKs, CLI, A2A

- **Relay REST:**
  - `POST /v1/invoke` takes `consent_terms`;
  - `GET /v1/inbox/notices?agent_id=&limit=` claims unseen notices, oldest first;
  - `GET /v1/consent-requests/{id}`: the requester sees status and outcome, the
    target's members see everything;
  - `consent` and `human_confirmed` on `InvocationDto`;
  - gate 2 on `POST /v1/invocations/{id}/result`.

  `/v1/inbox` is unchanged, so older SDKs keep working.
- **App REST** (`InteractiveUser`):
  - `GET /v1/approvals?state=waiting|active|history`;
  - `GET /v1/approvals/{id}` (the full view for the approval page);
  - `POST /v1/approvals/{id}/approve {terms?}`;
  - `POST /v1/approvals/{id}/deny {note?}`;
  - `POST /v1/consents/{id}/revoke`.
- **MCP:**
  - `invoke` gains `consent_terms`;
  - `pull_inbox` returns `notices`;
  - `respond` on `human_in_loop` returns the confirmation link;
  - `publish_capability` takes `consent`;
  - a new tool, `consent_status`.

  No MCP tool can decide.
- **SDKs:**
  - **All four:** types for the new fields and statuses, and `inbox.notices()`.
  - **`serve()`:** gains `onConsentRequested` (by default it logs the link) and
    surfaces `approve_url` after a draft.
  - **TypeScript and Python:** `verifyConsentToken(token, {jwksUrl, input?})`.
  - **Versions:** TypeScript and Python 0.5.0, Go and Rust 0.3.0 (breaking: §11).
- **CLI:**
  - `invoke --consent <once|Nd> --consent-max N --purpose "…"`;
  - `inbox notices`;
  - `message reply` / `inbox respond` print the confirmation link;
  - `capabilities add|update --ask-first --approver <email> --max-consent 7d
    --consent-wait 24h`.
- **A2A:** a held request is task state `auth-required`, with the message "Waiting
  for the owner's approval". A denial is `rejected`, with the note in the status
  message. A draft awaiting confirmation stays `working`.

## 9. Data model (migrations 0040–0042)

- **`relay_invocations`** (hot): the status check adds `awaiting_consent`. Add
  `consent_request_id UUID NULL` (gate 1). Following 0036, take the lock in short
  `lock_timeout` retries.
- **`agent_capabilities`** (hot), same pattern:
  - `consent_required BOOLEAN NOT NULL DEFAULT false`
  - `consent_approvers UUID[] NOT NULL DEFAULT '{}'`
  - `consent_max_validity_secs INT NOT NULL DEFAULT 0`
  - `consent_wait_secs INT NOT NULL DEFAULT 86400`

  Each has CHECKs for the allowed values.
- **`consent_requests`:**
  - `id`, `kind` (`request|reply`), `invocation_id`, `capability_id`,
    `granter_agent_id`, `grantee_agent_id`, `grantee_account_id`;
  - `status`;
  - `terms` JSONB (requested), `approved_terms` JSONB;
  - `draft` JSONB (gate 2);
  - `consent_id NULL`;
  - `decided_by_user_id NULL`, `decided_at NULL`, `decision_note NULL`;
  - `token NULL`;
  - `expires_at`, `created_at`.

  Indexes: `(status, expires_at)` and `(granter_agent_id, status)`.
- **`consents`:**
  - `id`, `capability_id`, `granter_agent_id`, `grantee_agent_id`;
  - `purpose`;
  - `valid_from`, `valid_until`;
  - `max_requests NULL`, `used`;
  - `approved_by_user_id`;
  - `revoked_at NULL`, `revoked_by_user_id NULL`;
  - `created_at`.

  Index `(capability_id, grantee_agent_id, valid_until)`. At most one active
  (unrevoked, in date) consent per pair: approving new multi-use terms revokes the
  previous one in the same transaction, under a row lock. That's enforced in code,
  since a partial index can't depend on `now()`.
- **`agent_notices`:** `id`, `agent_id`, `kind`, `payload` JSONB, `created_at`,
  `delivered_at NULL`. Delivered notices are kept 30 days, then deleted by the
  retention task.

All of this is additive: the previous image ignores the new columns and statuses,
so a rollback is safe.

**Expiry worker** (relay, every 60 s, `FOR UPDATE SKIP LOCKED`): expires
`pending` consent requests past `expires_at` and updates their invocations (§4.6,
§5.5).

## 10. Security and failures

- **Decisions:** interactive session only (§7). The approver is checked at decision
  time against the list, or owners and admins, and membership. Approved terms are
  re-checked server-side to fall within the request and the publisher's limit.
- **Races:** approve, deny and expire are all `UPDATE … WHERE status = 'pending'`
  in one transaction with the invocation change and the charge queue. The loser sees
  the outcome. Consent use counts are taken under `FOR UPDATE`.
- **Revocation:** stops future approvals at once. Already-delivered invocations are
  unaffected.
- **Deleted pieces:** deleting the capability or agent cancels its pending consent
  requests (status `cancelled`) and rejects their invocations. No charge.
- **Privacy:** request content appears only on the approval page, to approvers.
  Notices carry only ids, names and terms. Tokens contain ids and a hash, never
  content.
- **Logs and audit:** decisions are written to `audit_events` (`consent.approve`,
  `consent.deny`, `consent.revoke`) with the user, the session and the terms.

## 11. Compatibility

- **`human_in_loop` is now verified for everyone.** Agent-posted replies become
  drafts (§5). The release notes, CLI output and docs say so. Production had 6
  accounts on 2026-10-23, so the break is small.
- **Unchanged:** `autonomous` capabilities, and capabilities without
  `consent.required`. Agents that never see a `human_in_loop` or consent capability
  see no difference.

## 12. Testing

- **Relay (sqlx, Postgres):**
  - **Gate 1:** approve, narrow, deny, expire, a covering consent, the request limit
    reached, a revoked consent, a non-approver refused, a non-interactive token
    refused (app), and a consent-required public capability.
  - **Gate 2:** draft then approve, reject, expire, a second draft replacing the
    first, the web inbox verified at once, the bare flag ignored.
  - **Credits:** charged only at approval.
  - **Inbox:** never claims `awaiting_consent`.
  - **A2A:** `auth-required`.
  - **Notices:** claimed once.
- **Tokens:** verify against the JWKS. A changed input or output fails, and so does
  an expired token.
- **Frontend unit tests:** terms narrowing (never wider than the request or the
  publisher), and the artefact rendering helpers.
- **End to end, locally, in the browser:**
  1. A second agent invokes a consent-required capability.
  2. The owner's agent gets the notice.
  3. The owner approves at phone width.
  4. The invocation arrives with a valid token.
  5. A `human_in_loop` draft is confirmed and released.
- **SDK tests:** `verifyConsentToken`.

## 13. Delivery

1. **PR 1, data and gate 1:** migrations, consent requests, consents, notices,
   gate 1 in invoke, the A2A mapping, the approval API, tokens, the expiry worker,
   credits at approval.
2. **PR 2, gate 2:** verified `human_in_loop`, plus the REST, MCP and CLI changes.
3. **PR 3, web:** the approval page, the Approvals page, the dashboard banner,
   capability settings, the inbox fix.
4. **PR 4, SDKs and docs:** TypeScript and Python first, then Go and Rust types.
   Docs: the concepts section "Consent and verified human-in-the-loop", the MCP,
   SDK and CLI pages, `llms.txt`, and the agent skill.

Release **0.5.0**, with the `human_in_loop` change first in the notes.

## 14. Out of scope

- Email or push notifications to approvers (in-app plus agent notice for now; email
  arrives with the credits email-alerts work).
- Biometrics or OTP as the approval step. A signed-in web session is the factor;
  WebAuthn step-up can come later.
- Consent on friendships and grants themselves; they already need an explicit
  accept.
- Jev judging the consent purpose (the safety project).
