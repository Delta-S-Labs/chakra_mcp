# Waking pull agents W5: the owner answers on the web — implementation plan

Derived from `2026-10-28-waking-pull-agents-design.md` §9 ("The owner answers on the
web"). W5 is PR 5 of §15, and it's split in two:

- **W5a:** the backend, and the built-in page for self-hosted servers.
- **W5b:** chakramcp.com's web app, and the SDKs' `serve` warning.

W5b depends on W5a's API.

**Branches:** `feat/w5a-owner-reply`, then `feat/w5b-owner-reply-web`.

## W5a

### `shared::consent::reply::answer_as_owner`

```rust
pub async fn answer_as_owner(
    db, wake, checker: Option<&ComplianceChecker>, actor: Actor,
    invocation_id, draft: Draft,
) -> Result<OwnerAnswer, ApiError>
```

**Checks, in this order:**
1. The invocation exists. It doesn't, or the user can't see it: 404.
2. Its capability is `human_in_loop`. Otherwise the answer is 409: other capabilities
   need a claim, as before.
3. The user may decide for it (`approvers::may_decide`). Otherwise 403. The caller
   checks the session is interactive.
4. It's `pending` or `in_progress`. Settled is `chk.invocation.already_settled`, and
   `awaiting_consent` is a 409.

**Screening and the write:**
- **Screening:** the draft is screened on `Channel::Reply` when message safety is on, and
  the report is stored. A block refuses the reply and changes nothing.
- **One transaction:**
  - a `pending` row is claimed first (`in_progress`, `claimed_at`,
    `claimed_by_user_id` = owner, `deliveries + 1`);
  - the approved reply is stored and released;
  - `reply_confirmed` goes to the agent only if it had claimed the call.
- **On failure** nothing is committed.
- **What moves to `shared`:** `confirm_now`'s body, `store_reply_report` and
  `too_large_to_screen`. `confirm_now` goes.

### The relay's `submit_result`

- **A `human_in_loop` call from an interactive approver** goes to `answer_as_owner`
  before anything else. A pending call works too: the owner's web reply is the one
  exception to `not_claimed`.
- **Everyone else** follows W4's rules (`check_answerable`), is screened once, and gets a
  draft (`human_in_loop`) or finishes.

### `owner_reply_url`

- **Value:** `{frontend_base_url}/app/messages/{id}`, built in memory.
- **Where:** on `human_in_loop` invocations for the target side:
  - REST inbox pulls, and invocation reads when `i_served`;
  - MCP `pull_inbox` items, and `poll_invocation` when served.

### `GET /v1/approvals?state=messages`

- **What it lists:** `human_in_loop` invocations that are `pending` or `in_progress`,
  with no reply draft waiting. Only those the user may decide for, under the waiting
  list's approver rule, newest first.
- **Item shape:** invocation id, capability name, target agent, requester, `created_at`,
  `expires_at`, status, and `owner_reply_url`.
- **Why a new `state`, not the `waiting` list:** the current web app treats every
  waiting item as a consent request. W5b merges the two lists in its Waiting tab.

### `/app/messages/{id}` (built-in, `HOSTING_MODE=self_hosted`)

- **The page:** sign in, then the call. That's the caller, the capability, when it
  arrived and its deadline, and the input as fields, like the approval page.
- **The reply box:**
  - **plain text** when the capability has no output schema; the output is the text;
  - **message_owner's shape** when the schema has a `reply` string and a `status` enum
    with `replied`. The box is plain text, sent as
    `{status: "replied", reply, responded_at}`. Most owners answer `message_owner`
    calls, so they shouldn't have to type JSON;
  - **JSON** otherwise, checked before sending.
- **Decline** sends `failed` with an optional reason.
- **Who sees it:**
  - non-approvers and other accounts get 404;
  - a settled call shows how it ended, and a call with a draft waiting links to its
    approval page.
- **How it's built:** CSRF and the cookie session, no JavaScript, an askama template,
  like `/app/approvals/{id}`.

### Tests

- **`answer_as_owner`:**
  - a pending call is claimed and answered in one go, with no notice;
  - a claimed call gets `reply_confirmed`;
  - non-approvers get 403;
  - an autonomous capability is refused;
  - settled gets `already_settled`;
  - a screening block leaves the row pending;
  - a failure part-way commits nothing.
- **Relay:**
  - an approver's web session answers a pending `human_in_loop` call over REST;
  - an API key on the same pending call gets `not_claimed`;
  - an approver's key, not interactive, still gets a draft on a claimed call.
- **The `messages` list:**
  - it shows open `human_in_loop` calls to approvers only;
  - a drafted one is left out;
  - settled ones drop off.
- **`owner_reply_url`:** on inbox items and served reads, not on the caller's reads or on
  autonomous calls.
- **Built-in page:**
  - sign in, answer a pending call (text and message_owner shapes);
  - decline;
  - non-approver 404;
  - settled page;
  - bad JSON is refused;
  - CSRF.

## W5b

- **`/app/messages/[id]` in Next.js:** the same view and reply shapes. It posts to
  `/v1/invocations/{id}/result` with the web session.
- **Approvals → Waiting** shows messages (from `state=messages`) beside requests and
  drafts, linking to `/app/messages/{id}`.
- **Web Inbox:** the reply form also shows on inbound `pending` `human_in_loop` rows.
- **SDKs:**
  - `owner_reply_url` on `Invocation`;
  - `serve`'s warning for a `human_in_loop` call with no human handler prints the link.
- **Tests:** frontend unit tests for the reply shapes, and SDK tests for the field.
