# Credits P5 + P6: signals for agents and low-balance alerts

**Status:** approved in conversation, 2026-10-15, then reviewed. Phases P5 (alerts) and P6
(agent signaling) of `docs/specs/2026-09-23-credit-ledger-foundation-design.md` § Later phases.
They ship together because they share one new signal: "this account is running low".

## 1. Why

A refused call today says only "rate limit exceeded" or "insufficient credits", so an agent
either gives up or retries in a tight loop. And nobody learns an account is running low until
its calls start failing.

Decisions made with the user:

- **Out-of-credits refusals** tell the caller to wait until the next free monthly grant, and
  link to where credits can be bought. Clients that honour `Retry-After` then stop hammering,
  and a purchase still works at once for anyone who retries sooner.
- **Low-balance alerts are in-app only** for now: a banner across the app, plus a header that
  agents see. No email, no webhooks, no new service or secret.

## 2. The rules this keeps

- **Invocations never wait on the database.** Every new signal on the invocation path comes
  from the in-memory switches the credits worker refreshes every 5 s, or from the Redis count
  the rate limiter already takes ([[never-hamper-invocation-performance]]).
- **Fail open.** A stale switch snapshot or a Redis error adds no headers and refuses
  nothing, as today.
- **Additive.** Existing status codes, error codes and JSON fields stay. New fields and
  headers are added.

## 3. The "low" signal

- **Setting:** `CREDITS_LOW_BALANCE_MC`, in `CreditsConfig` (shared crate) and, like its
  siblings, as a top-level `server.toml` key named after the variable:
  `credits_low_balance_mc`. `CreditsFile` is flattened into the server config.
  - **Default:** `max(100000, cost_per_invocation_mc)`. 100,000 mc is 100 credits, about
    1,000 calls at the default 0.1 credit per call. The `max` means a server with a higher
    per-call cost never fails to boot after upgrading.
  - **High per-call costs:** a server whose cost is above 100,000 mc gets low = cost, so it
    never reports "low". That's by design, not a bug.
  - **Validation:** an explicitly set value must be at least the cost of one invocation.
    `from_sources` resolves it after `cost_per_invocation_mc`, since both the default and the
    check depend on the resolved cost.
  - **Compose** passes `${CREDITS_LOW_BALANCE_MC:-}`.
    - An empty value means unset, so the `max` default applies.
    - Writing `100000` there would make an explicit value that fails validation on servers
      with a higher per-call cost.
    - Older images don't read the variable at all, so the empty value can't hurt them.
  - **Documented in:** the `init` template (`backend/server/src/main.rs`),
    `infra/.env.example`, `docs/self-hosting/{compose,kubernetes}.md`, and the chart
    README's `extraEnv` note.
- **Definition:** an account is **low** when its wallet is not unlimited, has been granted,
  and `cost_per_invocation_mc ≤ balance_mc < low_balance_mc`. An account below the cost is
  **blocked**, as today, and is not also low.
- **Credits off:** `CreditsConfig::low_below_mc()` returns `i64::MIN` when credits are off,
  like `block_below_mc()`, so nothing is ever low. Granted wallets from before credits were
  turned off must not report "low".
- **Relay:** `CreditCache::refresh` takes `&CreditsConfig` (instead of a bare threshold) and
  replaces its blocked-accounts query with one query that returns both sets:

  ```sql
  SELECT account_id, balance_mc < $1 AS blocked
    FROM credit_wallets
   WHERE NOT unlimited AND free_grant_period IS NOT NULL AND balance_mc < $2
  ```

  where `$1` is `block_below_mc()` and `$2` is `max(low_below_mc(), block_below_mc())`.
  - The snapshot gains `low: HashSet<Uuid>` and `CreditCache::is_low(account)`,
    fresh-or-false like `is_blocked`.
  - **Call sites updated:** the credits worker (`relay/src/limits/credits/worker.rs`), the
    relay binary (`relay/src/main.rs`) and the combined server
    (`backend/server/src/main.rs`).
  - **sqlx cache:** the query moves from `query_scalar!` to `query!`, so the cache is
    regenerated.
- **Metric:** gauge `chakramcp_credits_low_accounts`, set on each refresh, next to the existing
  blocked-accounts gauge.
- **App:** the shared helper that decides "blocked" for the app gains a sibling `is_low`, so
  the app and the relay agree.

## 4. P6: what agents see

### 4.1 The limiter reports its count

`RateLimiter::check` returns the count it already reads, as a `RateOutcome`:

- `Allowed { limit, remaining, reset_secs }` or `Limited { limit, reset_secs }`;
- `Unknown` for the `Noop` limiter and for Redis errors (fail open, no headers).

`reset_secs` is the time left in the fixed one-minute window: `60 − (unix seconds mod 60)`,
at least 1. `remaining` is `limit − count`, floored at 0.

### 4.2 `enforce` returns a decision

`limits::enforce` returns a `LimitDecision`:

- `refusal: Option<Refusal>`, where `Refusal` is
  - `RateLimited { limit, retry_after_secs }`, or
  - `InsufficientCredits { retry_after_secs, next_grant_at }`;
- `rate: Option<RateInfo>` (`limit`, `remaining`, `reset_secs`) for headers on allowed calls;
- `credits_low: bool`.

- **`next_grant_at`:** 00:00 UTC on the 1st of next month, when the worker's monthly grant
  lands. It reuses `next_month_start()` (`relay/src/handlers/invoke.rs`), made `pub(crate)`.
- **`retry_after_secs` for credits:** the time until then, at least 1.
- **Its limits:** the grant may not cover the balance owed, or the account's grant may be
  overridden to 0, and the relay can't tell either from memory. The docs say to treat it as
  "not before", and that a purchase works at once.
- **Shadow mode** (`LIMITS_ENFORCE=false`) still logs and never refuses. An allowed call
  that is over its limit gets `RateLimit-Remaining: 0`.

**Plumbing.**
- `TrustedOutcome::Enqueued` carries the `RateInfo` and `credits_low` back to REST and MCP.
  The public path returns them directly.
- MCP's `call_tool` result carries them up to `handle`, which builds the HTTP response.
- A2A passes them to `deny_response` as an extra argument, rather than as fields on the
  `DenyReason` variants, which stay data-free and `PartialEq`.

### 4.3 Refusals, per surface

All three carry the same facts.

- **REST** (`/v1/invoke`, trusted and public):
  - `ApiError::RateLimited` and `ApiError::InsufficientCredits` gain the fields above.
    `IntoResponse` sets `Retry-After` for both, the way it already does for
    `SigninRateLimited`.
  - The body's `error` object gains `retry_after_secs`, plus `next_grant_at` and, in managed
    hosting mode only, `credits_url` for the credits case.
    - `credits_url` is `{frontend_base_url}/app/credits`.
    - A self-hosted server may have no web UI at that address, so it omits the link.
    - `RelayState` gains a precomputed `credits_url: Option<String>`, built once at startup
      from the hosting mode and `frontend_base_url`. Both mains pass the hosting settings to
      the relay state.
  - Codes, statuses and `retryable` are unchanged.
- **Per-capability public quota** (`monthly_quota_exhausted`, a 429 the public-invoke path
  already returns): adds `Retry-After` until its existing `resets_at`.
- **A2A:**
  - `deny_response` keeps codes -32007 / -32008 and HTTP 429.
  - The JSON-RPC `error.data` gains `retry_after_secs`, plus `next_grant_at` and
    (managed only) `credits_url` for credits, and the HTTP response sets `Retry-After`.
- **MCP** (the `invoke` tool, the only MCP tool that runs `enforce`): the tool result stays
  `isError: true` with a text message, now with the hint
  - Rate limit: "rate limit exceeded; retry in 23 s".
  - Credits, managed: "insufficient credits; the free grant arrives on 2026-11-01, or buy
    credits at …".
  - Credits, self-hosted: "insufficient credits; the free grant arrives on 2026-11-01".

  It also adds `structuredContent` with `code`, `retry_after_secs` and, for credits,
  `next_grant_at` and (managed only) `credits_url`. MCP has no HTTP 429 to hang a header on.

### 4.4 Headers on allowed calls

On a successful invocation response from REST (trusted and public) and A2A (push and pull):

- `RateLimit-Limit`, `RateLimit-Remaining`, `RateLimit-Reset`, when the limiter returned a
  count;
- `ChakraMCP-Credits: low`, when the caller's account is low.

MCP responses are JSON-RPC over one HTTP response, so the `invoke` tool's HTTP response gets
the same headers, and a low account's successful tool result also carries
`_meta.chakramcp/credits: "low"`.

**CORS:** the relay's CORS layer exposes only `WWW-Authenticate` today
(`backend/relay/src/lib.rs`). It adds `Retry-After`, `RateLimit-Limit`, `RateLimit-Remaining`,
`RateLimit-Reset` and `ChakraMCP-Credits` to `expose_headers`, so browser-based clients can read
them.

### 4.5 SDKs

- **All four SDKs** (TypeScript, Python, Go, Rust) gain typed errors:
  - `RateLimitedError`: code `account_rate_limited`;
  - `CreditsExhaustedError`: code `account_credits_exhausted`, with the next-grant time and
    the credits link (absent when the server omits it).
- **Every API error** exposes the retry delay (from `Retry-After`, else the body), and the
  existing `QuotaExhaustedError` gains it too.
- **Field naming** follows each SDK's existing quota error:
  - **TypeScript and Python:** snake_case data fields, like `resets_at` today
    (`retry_after_secs`, `next_grant_at`, `credits_url`).
  - **Go:** exported fields. The new errors wrap `*Error` and implement `Unwrap`, like
    `QuotaExhaustedError`, so `errors.As(err, &*Error)` keeps working.
  - **Rust:** new `Error` variants, plus a field on `Error::Api`. `Error` isn't
    `#[non_exhaustive]`, so this is a breaking change and the Rust SDK goes to 0.2.0.
- **No automatic retries:** a caller chooses what to do. An out-of-credits `Retry-After` can
  be weeks long, which can stall HTTP clients that honour it automatically (urllib3's
  `Retry`, for example). The SDK docs say so.
- **Releases:** each SDK's version bumps, released with its own workflow as part of the full
  release. They go out after the backend is deployed.

## 5. P5: what people see

- **API:**
  - `GET /v1/orgs/{slug}/credits` (`CreditsView`) gains `low: bool` (status stays
    `active` / `blocked` / `unlimited` / `off`, so no client's switch breaks) and
    `low_balance_mc`.
  - New `GET /v1/me/credit-alerts` returns, for each account the signed-in user belongs to
    whose wallet is low or blocked (credits enabled only):
    - `account_slug`, `account_name`;
    - `state` (`low` | `blocked`), `balance_mc`, `calls_left`;
    - `next_grant_on`.

    - Each row also has `account_type` (`individual` | `organization`), so the banner can
      say "Your personal account".
    - `calls_left` is clamped at 0.
    - The response has a top-level `purchase_available` (managed mode with purchasing
      configured, the same flag that fills `CreditsView.purchase`).
    - It's one query joining memberships and wallets, and is empty when credits are off.
    - Any interactive session can call it; API keys can too (it's read-only and scoped to
      the user's own accounts).
- **Banner** (`CreditAlertBanner`, in the app layout under the header):
  - It's a server component that fetches the alerts, wrapped in `Suspense` so it never
    delays the page. On a fetch error it renders nothing.
  - Layouts don't re-render on client navigation, so after a purchase settles, the buy box
    calls `router.refresh()`, which clears a banner that no longer applies.
  - **Low:** "Acme is low on credits: about 350 calls left." with a button: "Buy credits" when
    purchasing is available, else "View credits", linking to `/app/credits?account=acme`.
  - **Blocked:** "Acme is out of credits. Calls are refused until 1 Nov, or until you buy
    more." The "or until you buy more" part appears only when `purchase_available` is
    true.
  - Several accounts: one line each, at most three, then "and N more".
  - A low banner can be dismissed for the browser session. The dismissal is per-viewer
    (`sessionStorage`, guarded by try/catch) and comes back if the state changes. A blocked
    banner can't be dismissed.
- **Credits page:** a low notice above the balance, with the same wording.
- **Dashboard:** today the Credits stat card says "out of credits" only when purchasing is
  off; on chakramcp.com it always says "Buy credits →". The new order, first match wins:
  1. **blocked:** "Out of credits", with a Buy link when purchasing is available;
  2. **low:** "Running low", with a Buy link when purchasing is available;
  3. **otherwise:** as today.

## 6. Docs

- **Site docs:**
  - `/docs/sdk` and `/docs/mcp`: the headers, the error fields and the SDK error types.
  - `/docs/concepts`: a new "Limits and credits" section for REST (429s, rate limits,
    running out of credits). No page covers them today.
- **`frontend/public/llms.txt` and the agent skill:** how an agent should react to each
  refusal, including the long credits `Retry-After`. The skill's source is
  `.claude/skills/chakramcp-agent/SKILL.md`; `frontend/public/skills/` is regenerated from it
  by `sync-skills.mjs`.
- **INSTALL.md settings table:** `CREDITS_LOW_BALANCE_MC` (env and `server.toml` columns).

**Rollout:** the backend and app ship first; the SDK releases follow once production returns
the new fields.

## 7. Testing

- **Config unit tests:** the low threshold unset, explicit and valid, explicit and below the
  cost (refused), and a high-cost server on the default.
- **`credits_url`:** present in managed mode and absent in self-hosted mode, on REST and on
  A2A or MCP. `RelayState` gets a `with_credits_url` builder for the managed-mode tests.
- **Relay unit tests:**
  - `reset_secs` and `remaining` math;
  - `RateOutcome` from the counting limiter;
  - `enforce` decisions in enforce and shadow mode, including fail-open on `Unknown` and
    stale switches;
  - `next_grant_at` across month and year ends.
- **sqlx tests:**
  - the combined refresh query (low, blocked, unlimited, never granted);
  - REST and A2A refusals carry `Retry-After` and the body fields;
  - allowed calls carry `RateLimit-*` and `ChakraMCP-Credits`;
  - MCP's text, `structuredContent` and `_meta`.
- **App tests:**
  - `CreditsView.low`;
  - `/v1/me/credit-alerts`: only the user's accounts, low and blocked, empty when credits are
    off.
- **Frontend:** a unit test for the banner's wording and limits, plus a check in the dev
  server against a seeded low and blocked account.
- **SDKs:** each SDK's tests cover the two new errors and the retry delay, named per SDK:
  - TypeScript and Python: `retry_after_secs`;
  - Go: an exported field;
  - Rust: a struct field.

## 8. Out of scope

- Email, webhooks and request-a-higher-limit flows. P5's "request increase" half stays
  deferred.
- Exact balances on every response: only the low flag. The credits API has the balance.
- Automatic SDK retries.
