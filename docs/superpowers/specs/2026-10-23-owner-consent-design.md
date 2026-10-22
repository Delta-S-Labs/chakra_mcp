# Owner consent and verified human-in-the-loop

Status: approved design, 2026-10-23, revised after spec review. This is the first of
four projects from the owner's notes (Notion, "chakramcp", 2026-10-12). Agreed order:
**consent → safety → calls → MCP**.

1. **Consent** (this spec). An agent's owner verifiably approves requests before their
   agent sees them, and verifiably confirms human-written replies.
2. **Safety.** Jev (System One) checks extend from invocation inputs to outputs and
   messages, including off-platform contact detection and multi-language and encoded
   text.
3. **Calls.** Two-way sessions between agents. Each call has a memory mode (a shared,
   compacted call memory, or each agent keeping its own), a TTL on its data, and its
   audited history grouped together.
4. **MCP.** MCP becomes the front door for people and services: directory search,
   invoking public capabilities, and calls.

Calls will build consent and safety in from the start. MCP will expose all three.

## 1. Why

- **Nothing makes an agent ask its owner before acting.** For "book my calendar" or
  "pay this invoice", the owner wants to approve each request, or a bounded set of
  requests, before the agent acts. Today the agent sees a request as soon as it is
  queued.
- **`human_in_loop` is self-asserted.** A capability with
  `semantics = 'human_in_loop'` only needs the reply to carry `confirmed_by_human:
  true` (`report_result`, `backend/relay/src/handlers/invoke.rs` ~1220).
  - Any agent can send that flag. The CLI's `message reply` and `inbox respond` always
    set it, and MCP `respond` takes it as an argument (and checks only `succeeded`
    replies).
  - The flag isn't stored.
  - The web inbox can't answer these calls at all: it never sends the flag, so it
    gets a 409.

Both problems have the same fix: **a signed-in approver confirms, on the web, exactly
what they are agreeing to.**

## 2. Concepts

| | What the approver confirms | When | Capability setting |
|---|---|---|---|
| **Gate 1: request consent** | "Deliver this request to my agent" | Before the agent can see the request | `consent.required` |
| **Gate 2: verified reply** | "This reply is mine / I approve it" | Before the requester sees the reply | `semantics = 'human_in_loop'` |

The two gates are independent: a capability can use either, both or neither. Both
apply only to **pull-mode** agents in this version (§11).

- **Approver.** A user who may decide. That's the publisher's list, minus anyone who
  has since left the agent's account. If the list is empty, or everyone on it has
  left, it falls back to the account's owners and admins, so there is always someone
  who can decide. Resolved when the decision is made.
- **Consent request.** One record per held request (gate 1) or per draft reply
  (gate 2). Statuses: `pending` → `approved` | `denied` | `expired` | `cancelled`.
- **Consent.** A standing approval for one requesting agent on one capability, with a
  validity window and an optional request limit. It's created when an approver
  approves multi-use terms. While it lasts, it covers every request from that agent
  on that capability, whatever purpose or terms the new request proposes.
- **Consent terms.** What the requester proposes: a purpose, a validity (`once`,
  `<n>h` or `<n>d`) and an optional request limit. The model is India's Account
  Aggregator consent: the requester proposes, and the approver accepts, narrows or
  refuses. The approver can't widen the terms.
- **Consent token.** A signed JWT saying what was approved, bound to the exact
  delivered input or released output by its SHA-256.

## 3. Capability settings

Set when publishing or updating a capability, through REST, MCP, the CLI or the web:

| Field | Meaning | Default and limits |
|---|---|---|
| `consent.required` | Gate 1 on | `false`. Refused on push-mode agents (§11). |
| `consent.approvers` | User ids who may decide, for both gates | Empty means owners and admins. Each must be a member of the agent's account when set. |
| `consent.max_validity` | Longest consent an approver may grant | `0`, meaning each request on its own. Choose from `0`, `1h`, `1d`, `7d`, `30d`. |
| `consent.wait` | How long a held request or draft waits for a decision | `24h`, between `5m` and `7d` |

`semantics` keeps its two values. `human_in_loop` is now verified through gate 2 (§5)
and uses `consent.approvers` and `consent.wait`.

For consent-required capabilities, an input whose serialized JSON is larger than
16 KiB is refused with `413 chk.request.too_large`. That way the approver sees the
whole input and the agent receives it untruncated, since previews over 16 KiB become a
truncation marker (`truncate_for_audit`). A2A already refuses bodies over 16 KB in
`park`. Gate 2 has the same limit for drafts (§5).

## 4. Gate 1: consent before delivery

### 4.1 Invoking

1. **The requester invokes** through REST `POST /v1/invoke`, MCP `invoke`, A2A
   `SendMessage` or an SDK, optionally with
   `consent_terms: {purpose?, valid_for: "once" | "<n>h" | "<n>d", max_requests?}`.
   - **Where the terms go.** For REST, MCP and the SDKs, `consent_terms` is a body
     field. For A2A, it's `params.metadata["chakramcp.consent_terms"]`, with the same
     JSON shape, next to the existing `X-ChakraMCP-Capability` header.
   - The existing checks run first: the grant or public path, rate and credit limits,
     and Jev on the input.
   - Terms beyond `consent.max_validity` are capped. With no terms, the request is a
     single request (`once`).
2. **Still one round trip per path.** Three insert statements create invocations,
   each a CTE that writes the invocation and queues its credit charge:
   - `authorize_and_enqueue_trusted` (REST and MCP, `invoke.rs` ~696);
   - `invoke_public` (`invoke.rs` ~903);
   - `inbox_bridge::park` (A2A, ~127). It stores the raw SendMessage `params` as the
     input.

   **All three get the same consent CTE**, so that no path can skip it.
   - `sqlx::query!` accepts only string literals, optionally joined with `+`, so a
     `const` or `format!` fragment won't compile.
   - Instead, a `macro_rules!` wrapper splices the CTE text into each call's literal.
     The queries stay checked, and stay in the offline cache that the production
     build needs.

   For a consent-required capability, the CTE is extended:
   - **Use a covering consent.** One conditional statement:
     `UPDATE consents SET used = used + 1 WHERE capability_id = … AND
     grantee_agent_id = … AND revoked_at IS NULL AND valid_until > now() AND
     (max_requests IS NULL OR used < max_requests) RETURNING id`. There is no
     `FOR UPDATE`, and two calls racing for the last use simply fail the condition.
   - **If a consent was used:** the invocation is inserted `pending`, an `approved`
     consent request linking the consent is recorded, and the charge is queued. The
     call is delivered as usual.
   - **If not:** the invocation is inserted `awaiting_consent`, with no charge queued.
     The same statement inserts the `pending` consent request (`kind = 'request'`,
     the capped terms, `expires_at = now() + wait`) and an `agent_notices` row of
     kind `consent_requested`.

   Capabilities that don't require consent run exactly as today: the capability row
   is already loaded on every path, so there is no extra work. No token is signed
   while invoking (§6).
3. **The requester gets** `202 {status: "awaiting_consent", invocation_id,
   consent_request_id}`. The approval page needs an approver's sign-in, so the id
   reveals nothing.

### 4.2 Notifying the owner

The owner's agent pulls its notices with `GET /v1/inbox/notices`. A
`consent_requested` notice carries:
- the consent request id;
- the requesting agent and its account (display names, ids and verified flag);
- the capability name;
- the terms;
- `approve_url` (`{frontend}/app/approvals/{id}`) and `expires_at`.

It **never** includes the request's input. The agent passes the link to its human.

### 4.3 Hiding held content

While an invocation is `awaiting_consent`, and permanently if its request is denied,
expires or is cancelled, every target-side read returns the row's metadata but
replaces `input_preview` with `{"__chakramcp_held__": true}`. Target-side reads
include:
- REST `GET /v1/invocations` and `GET /v1/invocations/{id}`;
- MCP `list_invocations` and `poll_invocation`;
- the audit page.

**The rule: the input is hidden whenever the reader is a member of the target agent's
account (`i_served` in `fetch_one` and `list`), even if the reader is also on the
requesting side.**
- This covers one user in both accounts, and two agents in one account calling each
  other. Otherwise the target's credentials could read held input with
  `direction=outbound`.
- A requester outside the target's account still sees what it sent.

Only approvers see the held input, on the approval page (§7).

### 4.4 The decision

An approver decides in an interactive web session (§7).

**Approve,** as proposed or narrowed. In one transaction:
1. **Re-check authorization.** The original path must still hold:
   - for a friend call: the grant is active and unexpired, and its friendship is
     still accepted;
   - for a public call: the capability is still `public_invoke` and network-visible;
   - the capability still exists (capabilities are hard-deleted, which sets the
     invocation's `capability_id` to NULL);
   - both agents and both accounts are live (`tombstoned_at IS NULL`).

   If any check fails, the consent request becomes `cancelled`, the invocation is
   `rejected` with "No longer authorized: …", and the approver is told why. Credits
   aren't re-checked: the charge is queued and settled in the background as usual,
   and slight overshoot is acceptable.
2. **Close the request.** The consent request becomes `approved`, with the decider,
   the time and the final terms.
3. **Store standing consent.** Multi-use terms create a `consents` row with
   `used = 1`, revoking any earlier unrevoked consent for the same pair.
4. **Release the invocation.** It becomes `pending`, so the inbox claims it as usual,
   and its charge is queued.

**Deny,** with an optional note of up to 500 characters. The consent request becomes
`denied`. The invocation is `rejected` with `error_message = "The owner declined:
<note>"`, and no charge is queued.

### 4.5 Expiry and delivery

- **Expiry.** The expiry worker (§9) marks the request `expired` and the invocation
  `rejected` ("Consent request expired"). No charge is queued.
- **Delivery.** `InvocationDto.consent = {consent_request_id, consent_id?,
  approved_at, terms, token}` appears on inbox pulls (REST `/v1/inbox` and MCP
  `pull_inbox`) and on reads by either side after approval. `token` is null until it
  is minted (§6).

The inbox claim queries select only `status = 'pending'`, so held rows are never
claimed.

### 4.6 Other code that must learn the new status

- **Revoking a grant.** The cascade in `grants.rs` (~482) rejects the grant's
  `pending` invocations. It also rejects `awaiting_consent` ones and cancels their
  consent requests.
- **Friendships.** Nothing can end an accepted friendship today (`friendships.rs`
  `cancel` only works on proposed ones), so there is no cascade to add. If a
  friendship stops being accepted, the approval re-check (§4.4) catches it.
- **Reviews.** Eligibility in `reviews.rs` (~243) counts any invocation that isn't
  `rejected`. It must also exclude `awaiting_consent`, so a held request doesn't let
  the requester review an agent whose owner never approved the call.
- **A stale comment.** The comment in `policy/mod.rs` about a future
  `ConsentRequired` deny variant is superseded by this design and is updated.

## 5. Gate 2: verified replies (`human_in_loop`, pull-mode agents)

A single shared function, `submit_result`, is used by both REST
`POST /v1/invocations/{id}/result` and MCP `respond`. This replaces MCP's separate and
weaker check.

1. **Agent replies become drafts.** A reply to a `human_in_loop` capability from any
   non-interactive credential (an API key, an OAuth or device token, MCP or the CLI)
   is stored as a draft (status plus output or error) on the invocation's single
   `kind = 'reply'` consent request. The invocation stays `in_progress`.
   - The response is `202 {status: "awaiting_confirmation", consent_request_id,
     approve_url, expires_at}`.
   - `confirmed_by_human` is ignored.
   - **Size limit.** A draft whose serialized JSON is over 16 KiB is refused with
     `413 chk.result.too_large`, so the approver approves exactly what the requester
     will receive. Today REST truncates outputs at 16 KiB and MCP doesn't truncate.
2. **The approver decides** on the approval page, which shows the original request
   and the draft.
   - **Approve** releases the draft. The invocation becomes `succeeded` or `failed`,
     as drafted, with `human_confirmed = {consent_request_id, confirmed_at, token}`.
     The token is bound to the SHA-256 of the released output.
   - **Reject,** with an optional note: the invocation fails with "The owner declined
     to answer: <note>".
3. **Drafts are versioned.** A new draft before the decision replaces the old one, and
   the page says so. Approve sends the `draft_sha256` the approver was shown, and a
   mismatch returns `409 "The draft changed, review it again"`.
4. **Replies from the web inbox.** The relay recognises a signed-in session (§7).
   - **From an approver:** verified straight away. The reply becomes the draft,
     replacing any pending draft from the agent. The reply consent request is
     `approved` for that user in the same step, and the reply is released.
   - **From a member who isn't an approver:** stored as a draft, like an agent's
     reply, for an approver to confirm. Otherwise `consent.approvers` would mean
     nothing for gate 2.
5. **Expiry.** An unconfirmed draft expires and the invocation fails with "Reply not
   confirmed in time".

The agent learns the outcome from notices: `reply_confirmed`, `reply_rejected` and
`reply_expired`.

## 6. Consent token

The token is an EdDSA JWT signed with the relay's existing signing key
(`relay_signing_keys`, published as a `kid` at `/.well-known/jwks.json`). The active
key is cached in memory and re-read every 5 minutes. Nothing rotates keys in
production yet, so this is just enough to notice a future rotation.

| Claim | Value |
|---|---|
| header `typ` | `chakramcp-consent+jwt`. Verifiers must check it, because the same key signs the 60-second push bearer JWTs (`jwt_mint.rs`) and Agent Card JWS. |
| `iss` | The relay's public URL |
| `jti` | The consent request id |
| `kind` | `request` or `reply` |
| `inv` | The invocation id |
| `cap` | The capability id |
| `granter` / `grantee` | Target and requesting agent ids |
| `approver` | The approving user's id |
| `consent` | The consent id, when a standing consent covered the request |
| `sha256` | Hex SHA-256 of the RFC 8785 (JCS) canonical JSON of the **value delivered**: the `input_preview` the agent receives (`request`), or the released output (`reply`) |
| `iat`, `exp` | For `request`: `iat` + 24 hours. For `reply`: `iat` + 30 days. |

**Minting.** Each token is minted once and stored on its consent request (`token`).
- **Request tokens** are minted when the inbox first delivers the invocation, whether
  an approver approved it or a standing consent covered it. That way a pull agent
  that polls rarely never receives an already-expired token. It costs one extra
  write, paid only by consent-required invocations.
- **Reply tokens** are minted at release.

Minting never happens on the invocation path.

**Keys.** Retired signing keys stay in the JWKS for at least 30 days, the longest a
token lives.

**What it grants:** nothing in the relay. The owner's own systems can check it
offline to confirm that this exact input was approved.

## 7. Web

- **`/app/approvals/{id}`** is the link the agent passes on. It's built for phones and
  laid out like an Account Aggregator consent screen:
  - who's asking (agent, account and verified badge);
  - the capability (name, description, and your agent);
  - why (the requester's purpose and the grant's purpose);
  - what they're sending: the input rendered as fields, or for gate 2 the request
    plus the draft reply;
  - for how long and how often;
  - when it expires.

  The approver can approve as asked, narrow the terms (the options never exceed the
  request or the publisher's limit) or deny with a note. Non-approvers see who can
  approve. Requests that are already decided, expired, cancelled or revoked show
  their outcome.
- **`/app/approvals`** has three lists:
  - **Waiting:** what you can decide, with time left;
  - **Active consents:** requester, capability, valid until, uses n/M, with Revoke;
  - **History:** the last 90 days.

  The nav shows a count, and the dashboard shows a banner when anything is waiting.
- **Capability settings.** The agent's Capabilities panel gets an **Approvals**
  section, shown when either gate is on, with approvers and wait. Under it sit "Ask
  before delivering requests" (with the longest consent) and "A person writes the
  reply" (`human_in_loop`). Push-mode agents see why "Ask before…" isn't available.
- **The web inbox** answers `human_in_loop` calls: an approver's reply is verified
  straight away, and anyone else's becomes a draft (§5.4). It also shows `consent`
  and `human_confirmed` on invocations.

**Interactive sessions only.**
- The approval API lives in the **relay**, next to invocations, signing and the expiry
  worker, so releases record the usual invocation metrics.
- The app's delegated-token check (`is_delegated_token` in `backend/app/src/auth.rs`,
  used by `InteractiveUser` and `AdminUser`) moves to `chakramcp_shared`.
- The relay gets its own `InteractiveUser` extractor, built on that check. It refuses
  API keys and delegated (OAuth or device) tokens. The two crates keep separate
  extractors, because their state types and `AuthUser` structs differ.
- As a result, an agent holding its owner's API key can't approve its own requests.

## 8. API, MCP, SDKs, CLI and A2A

**Relay REST**
- `POST /v1/invoke` takes `consent_terms`.
- `GET /v1/inbox/notices?agent_id=&limit=` claims undelivered notices, oldest first.
  `/v1/inbox` is unchanged.
- `GET /v1/consent-requests/{id}` returns status, outcome and terms to the requester
  and to the target's members, never the held input.
- `InvocationDto` gains `consent` and `human_confirmed`.
- Gate 2 runs in `submit_result`.

**Relay REST, interactive sessions only**
- `GET /v1/approvals?state=waiting|active|history`
- `GET /v1/approvals/{id}`: the full view, including the held input or the draft, for
  approvers
- `POST /v1/approvals/{id}/approve {terms?, draft_sha256?}`
- `POST /v1/approvals/{id}/deny {note?}`
- `POST /v1/consents/{id}/revoke`

**MCP**
- `invoke` gains `consent_terms`.
- A **new tool, `pull_notices`**. `pull_inbox` keeps returning its array, and its
  items gain `consent`.
- `respond` uses `submit_result`, so it returns the confirmation link for
  `human_in_loop`.
- `publish_capability` takes `consent`.
- A new tool, `consent_status`.
- No MCP tool can approve or deny.

**SDKs**
- **All four:** types for the new fields and statuses, plus `inbox.notices()`.
- **`serve()`:** adds `onConsentRequested` (by default it logs the link), and the
  human-in-the-loop handler receives `approve_url` after posting a draft.
- **TypeScript and Python:** `verifyConsentToken(token, {jwksUrl, value?})`. It checks
  `typ`, the signature, the expiry and, when `value` is given, the JCS SHA-256.
- **Versions.** Rust 0.3.0 ships with PR 1, before gate 1 goes live (§11). It adds
  the new statuses, a `#[serde(other)] Unknown` variant, and `#[non_exhaustive]` on
  the status enum. With PR 4, the full SDK work ships as TypeScript and Python 0.5.0,
  Go v0.3.0 and Rust 0.4.0. These are breaking changes (§11).

**CLI**
- `invoke --consent <once|Nh|Nd> --consent-max N --purpose "…"`
- `inbox notices`
- `message reply` and `inbox respond` print the confirmation link.
- `capabilities add|update --ask-first --approver <email> --max-consent 7d
  --consent-wait 24h`

**A2A (pull-mode targets)**
- Terms go in `params.metadata["chakramcp.consent_terms"]` (§4.1).
- A held request is task state `auth-required`, with the message "Waiting for the
  owner's approval".
- A denied or cancelled request is `rejected`, with the note in the status message.
- A draft waiting for confirmation stays `working`.
- `inbox_bridge::get_task` and park learn the new statuses. Today unknown statuses map
  to `"unknown"`.

**Status allow-lists to update**
- `invoke.rs` list filter (~1282)
- `mcp.rs` (~317, ~803)
- `inbox_bridge::get_task`
- the CLI `invocations --status` filter (`backend/cli/src/commands/invocations.rs:22`)
  and the `invoke --wait` terminal check (`backend/cli/src/commands/invoke.rs:225`)
- the status enums in all four SDKs
- the frontend `InvocationStatus` type and badges

## 9. Data model (migrations 0040–0043)

sqlx runs each migration file in one transaction, so a lock taken in a file is held
until that file commits. Each busy table therefore gets its own short file, and the
scans happen in a file of their own.

| File | Contents | Locks |
|---|---|---|
| 0040 | The new tables `consent_requests`, `consents` and `agent_notices` | New tables only |
| 0041 | `agent_capabilities`: new columns and their CHECKs, added `NOT VALID` | `ACCESS EXCLUSIVE` on `agent_capabilities`, taken in the 0036 retry loop (100 ms tries, backing off). Catalog-only. |
| 0042 | `relay_invocations`: drop `relay_invocations_status_check` and add the replacement (adding `awaiting_consent`) `NOT VALID` | The same, on `relay_invocations` |
| 0043 | `VALIDATE CONSTRAINT` for each CHECK from 0041 and 0042 | `SHARE UPDATE EXCLUSIVE`: scans the table while reads and writes continue |

**`agent_capabilities` columns**
- `consent_required BOOLEAN NOT NULL DEFAULT false`
- `consent_approvers UUID[] NOT NULL DEFAULT '{}'`
- `consent_max_validity_secs INT NOT NULL DEFAULT 0`
- `consent_wait_secs INT NOT NULL DEFAULT 86400`

The defaults are constants, so adding the columns rewrites nothing.

**`relay_invocations`** gains no column. An invocation's consent requests are found
through `consent_requests(invocation_id, kind)`, which is indexed and joined on read.

**`consent_requests`**
- Identity:
  - `id`, `kind` (`request|reply`), `invocation_id`;
  - `capability_id`, `granter_agent_id`, `grantee_agent_id`, `grantee_account_id`.
- `status`.
- Terms and draft:
  - `terms` JSONB (requested), `approved_terms` JSONB;
  - `draft` JSONB and `draft_sha256` (gate 2);
  - `consent_id` (nullable).
- Decision: `decided_by_user_id`, `decided_at`, `decision_note`, `token`, all
  nullable.
- `expires_at`, `created_at`.

Indexes: `(status, expires_at)`, `(granter_agent_id, status)`, and a unique index on
`(invocation_id, kind)`, so each invocation has at most one request of each kind.

**`consents`**
- `id`, `capability_id`, `granter_agent_id`, `grantee_agent_id`, `purpose`.
- `valid_from`, `valid_until`, `max_requests` (nullable), `used`.
- `approved_by_user_id`, and `revoked_at` / `revoked_by_user_id` (nullable).
- `created_at`.

A partial unique index on `(capability_id, grantee_agent_id) WHERE revoked_at IS NULL`
keeps one unrevoked consent per pair. A new approval revokes the previous one, even an
expired one.

**`agent_notices`**: `id`, `agent_id`, `kind`, `payload` JSONB, `created_at`, and a
nullable `delivered_at`. The expiry worker deletes notices older than 30 days,
delivered or not, so agents that never pull notices don't accumulate them. The usage
retention task isn't used, because it doesn't run when `USAGE_RETENTION_DAYS=0`.

**Foreign keys.** The new tables have **no foreign keys to busy tables**
(`relay_invocations`, `agents`, `agent_capabilities`, `accounts`): adding one would
lock writers. Deleting a capability or agent is handled in code. Its pending consent
requests become `cancelled` and their invocations `rejected`, with no charge. Their
history is kept.

**Expiry worker.** It runs in the relay every 60 seconds, using
`FOR UPDATE SKIP LOCKED`. It's started from both `backend/relay/src/main.rs` and
`backend/server/src/main.rs`. It expires `pending` consent requests past `expires_at`
(§4.5, §5.5) and prunes old notices. It ships in PR 1.

**Rollback.** Every binary runs migrations at boot and refuses to start if the
database has a migration it doesn't know. A rollback therefore reverts code but
**keeps** migrations 0040–0043, following the project rule. Because the expiry worker
ships with PR 1, held rows always expire. A full rollback past PR 1 would need a
follow-up migration that rejects any remaining `awaiting_consent` rows.

## 10. Security and failures

- **Who decides.** Interactive session only (§7). The approver is resolved when the
  decision is made, as described in §2. The approved terms are checked again on the
  server: they must fall within the request and within the publisher's limit.
  Approval re-checks authorization (§4.4).
- **Races.**
  - Approve, deny, expire and cancel all use `UPDATE … WHERE status = 'pending'`,
    in one transaction with the invocation change and the charge queue. Whoever
    loses the race sees the outcome.
  - Consent uses are taken by the conditional update in §4.1.
  - A draft approval checks `draft_sha256` (§5.3).
- **Revocation.** Revoking a consent stops it covering new requests at once.
  Requests it already admitted are unaffected, whether delivered or not.
- **Privacy.**
  - Held input is hidden from the target (§4.3), and is shown only to approvers on
    the approval page.
  - Notices carry ids, names and terms only.
  - Tokens carry ids and a hash.
- **Audit.** Decisions are written to `audit_events` as `consent.approve`,
  `consent.deny`, `consent.cancel` and `consent.revoke`, with the user, the session
  and the terms.

## 11. Compatibility and limits

- **`human_in_loop` is now verified for pull-mode agents.**
  - Replies posted by agents become drafts (§5).
  - Production had 6 accounts on 2026-10-23.
  - The release notes, the CLI output and the docs all say so.
- **Older SDKs.**
  - **TypeScript and Python** return the new bodies as they are.
  - **Go (v0.2.0 and earlier)** decodes leniently. After a draft is stored,
    `Respond` returns an invocation with an empty `ID` and status
    `awaiting_confirmation`, and `Serve` carries on as if the reply went through.
    That's harmless, but nothing shows the confirmation link.
  - **Rust (0.2.0 and earlier)** has no catch-all `InvocationStatus`. Every response
    carrying a new status fails to parse:
    - the `202` draft body;
    - an invoke of a consent-required capability, which fails **after** the
      invocation was created, so retrying creates duplicate consent requests;
    - any list, get or poll that includes an `awaiting_consent` row.

    Rust 0.3.0 adds the statuses and a `#[serde(other)] Unknown` variant, so that
    future statuses don't break it. It is released with PR 1, before any capability
    can require consent, and Rust users need it. Go v0.3.0 reads the new fields.
  - The release notes say all this.
- **Push-mode agents.**
  - The relay forwards A2A calls to them at once, so it can't hold a call or a reply.
  - `consent.required` is refused when publishing on a push-mode agent, and an agent
    with any consent-required capability can't switch to push mode.
  - `human_in_loop` on push agents stays unverified (the reply comes straight from
    the agent's endpoint). The docs say so.
  - Holding requests for push agents and forwarding them after approval is future
    work.
- **Everything else is unchanged:** `autonomous` capabilities, and capabilities
  without `consent.required`.

## 12. Testing

**Relay tests (sqlx, against Postgres)**
- **Gate 1:**
  - approve, narrow, deny and expire;
  - a standing consent covers a request;
  - the request limit is reached, including two callers racing for the last use;
  - a revoked consent stops covering;
  - a non-approver is refused, and so is a non-interactive token;
  - the approver list falls back when everyone on it has left;
  - approval after the grant is revoked cancels the request;
  - the trusted, public and A2A paths all hold requests (one test each);
  - inputs over 16 KiB are refused;
  - held input is hidden from REST and MCP reads, including when the same user owns
    both agents;
  - revoking a grant rejects its held invocations and cancels their requests;
  - a held request doesn't make the requester eligible to review.
- **Gate 2:**
  - a draft is approved, rejected or expires;
  - a replaced draft returns 409 on a stale hash;
  - drafts over 16 KiB are refused;
  - an approver's web reply is verified straight away and replaces the agent's draft;
  - a non-approver's web reply becomes a draft;
  - the bare `confirmed_by_human` flag is ignored;
  - MCP `respond` runs the same path.
- **Elsewhere:**
  - credits are charged only on approval;
  - the inbox never claims `awaiting_consent`;
  - A2A shows `auth-required`;
  - notices are claimed once;
  - `consent.required` is refused on push agents.

**Other tests**
- **Tokens:** verify against the JWKS. A changed value fails, and so do a wrong `typ`
  and an expired token.
- **Frontend unit tests:** terms narrowing (never wider than the request or the
  publisher's limit), and the helpers that render the consent screen.
- **End to end,** locally in the browser:
  1. A second agent invokes.
  2. The owner's agent gets the notice.
  3. The owner approves at phone width.
  4. The invocation arrives with a valid token.
  5. A `human_in_loop` draft is confirmed and released.
- **SDKs:** `verifyConsentToken`, with JCS test vectors shared across TypeScript,
  Python and Rust.

## 13. Delivery

Four PRs, each with its own implementation plan. CD deploys every merge to
chakramcp.com, so **no PR may leave a half-built flow live**. A gate turns on only in
the PR that ships its web page.

1. **Gate 1 backend, dark.** Nothing anyone can see changes:
   - migrations 0040–0043;
   - the shared interactive check;
   - consent requests, consents and notices;
   - the consent CTE in all three insert paths, and hidden held input;
   - A2A `auth-required`;
   - the approval API;
   - tokens with the cached key;
   - the expiry worker;
   - credits at approval;
   - the cascades in §4.6;
   - the push-mode refusal;
   - Rust SDK 0.3.0 with the new statuses (§8), released before PR 2 merges.

   The publish and update APIs don't accept `consent` yet, so `consent.required` can
   only be set by tests.
2. **Gate 1 live.** The web side and the inputs that turn it on:
   - web: the approval page, the Approvals page, the dashboard banner, and the
     Approvals section of capability settings (approvers, wait, and "Ask before
     delivering requests");
   - `consent` on REST, MCP and CLI publish and update;
   - `consent_terms` on REST, MCP, CLI and A2A invoke;
   - `GET /v1/inbox/notices`, MCP `pull_notices` and `consent_status`, and CLI
     `inbox notices`.
3. **Gate 2, all at once,** so `human_in_loop` calls never break in production:
   - `submit_result`, shared by REST and MCP, and verified `human_in_loop`;
   - the draft view on the approval page;
   - the web inbox fix, and "A person writes the reply" in the Approvals section;
   - CLI output with the confirmation link.
4. **SDKs, examples and docs:**
   - SDKs: TypeScript and Python first, then Go and the rest of Rust.
   - The HITL examples: `examples/workers/pull-worker-with-hitl.ts`,
     `pull_worker_with_hitl.py` and `examples/voice-agent-demo`.
   - Docs: a concepts section ("Consent and verified human-in-the-loop"), the MCP,
     SDK and CLI pages, `llms.txt`, and the agent skill.

Release **0.5.0**, with the `human_in_loop` change first in the notes.

## 14. Out of scope

- Email and push notifications to approvers. For now there's the in-app page plus the
  agent's notice. Email comes with the credits email-alerts work.
- Biometrics or OTP as the approval step. The signed-in web session is the factor;
  WebAuthn step-up can come later.
- Holding and forwarding calls to push-mode agents.
- Jev judging the consent purpose. That's the safety project.
