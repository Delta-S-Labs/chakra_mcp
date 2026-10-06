# Credits P4: buying credits through Dodo Payments

**Status:** design approved 2026-10-06; plan: `2026-10-06-credits-p4-purchasing-plan.md`
**Date:** 2026-10-06
**Builds on:**
- `docs/specs/2026-09-23-credit-ledger-foundation-design.md`: the ledger, the wallets and the async worker (P1), the owner view (P2) and admin management (P3). P4 is its "Purchasing (managed-only, Dodo)" phase.
- `2026-10-01-self-hosting-phase3-design.md`: `HOSTING_MODE`. Purchasing exists only on a `managed` server.

## 1. Goal

On chakramcp.com, any member of an account can buy credits for it. They type any amount from $1 to $5,000, pay in Dodo's checkout over `/app/credits` without leaving the app, and see the credits arrive within seconds. Payments, credits and usage are easy to find in the web UI. The monthly free grant rises from 100 to 5,000 credits.

Self-hosted servers don't change. They have no purchase routes and no buy UI, and there's nothing new to configure.

**Not in scope:**
- refunds, disputes and chargebacks beyond logging them (they stay manual, as P1 decided);
- subscriptions or automatic top-ups;
- coupons;
- prices in currencies other than USD;
- buying from the CLI, an SDK or an agent;
- cost weighting and a free-balance cap (later phases of the credits spec).

## 2. Decisions (approved 2026-10-02 to 10-06)

| Decision | Choice |
|---|---|
| Amount | Any amount, $1 to $5,000 per purchase (settings) |
| Price | $1 = 1,000 credits; a call stays at 0.1 credit, so $1 buys 10,000 calls (setting) |
| Who buys | Any member of the account, from an interactive web session only. API keys, and OAuth, device-pairing and CLI tokens, can't start a checkout |
| Checkout | Dodo's overlay over `/app/credits`. Our server creates every checkout session and records it first |
| Crediting | Only Dodo's signed webhook, and only for a payment whose `checkout_session_id` is one of our checkouts |
| Free grant | 5,000 credits a month, as the new default |
| Existing accounts | Each gets 5,000 at its first grant (§3.2) |
| Refunds and disputes | Manual: a refund in Dodo, plus an `adjustment` in the admin console |
| Where | Only when the server is `managed`, credits are on and Dodo is configured (§9) |

## 3. Pricing and the free grant

### 3.1 Price

- **Credits for an amount:** `credits_mc = amount_cents × credits_per_usd × 10`. One credit is 1,000 mc and one dollar is 100 cents.
  - At the default 1,000 credits per dollar, a cent buys 10 credits (10,000 mc).
  - It's integer math, with no floats, like the rest of the ledger.
- **Limits:**
  - `100 ≤ amount_cents ≤ 500_000` (settings).
  - The biggest purchase is 5 million credits (5 × 10⁹ mc), well under `MAX_AMOUNT_MC` (10¹²).
- **Changing the price:** a checkout's credits are fixed when it's created, so a price change only affects later checkouts.
- **Tax:** Dodo is the merchant of record and handles it according to the product's tax settings. Credits always come from the amount we set, before tax.

### 3.2 Free grant

- **The new default:** `CreditsConfig`'s default `default_monthly_free_mc` goes from `100_000` to `5_000_000` (`backend/shared/src/credits.rs`), and Compose's `${CREDITS_DEFAULT_MONTHLY_FREE_MC:-100000}` changes to match.
  - The docs change too: the INSTALL.md settings table, `compose.md` and the website's credits copy. The chart doesn't set it.
  - Production's `.env` doesn't override it, so chakramcp.com follows the default.
  - Self-hosters who turn credits on get 5,000 too.
- **Existing accounts:** wallets are created lazily, and the first grant comes with the wallet (worker `grant_rows`). On 2026-10-03 no account on production had received a grant yet, so every account's first grant is the new 5,000.
  - If someone invokes before the deploy and gets the old 100, the deploy checklist (§12) tops them up with the existing `chakramcp-server credits grant`.
  - No new command is needed.
- **Per-account overrides** (`monthly_free_grant_mc`, set by an admin) keep winning, as today.

## 4. Data: `credit_checkouts` (migration 0038)

```sql
CREATE TABLE credit_checkouts (
    id               UUID        PRIMARY KEY,
    account_id       UUID        NOT NULL,      -- no FK, like credit_ledger: outlives a deleted org
    user_id          UUID        REFERENCES users(id) ON DELETE SET NULL,
    buyer_email      TEXT        NOT NULL,      -- for the payments list, even after the user goes
    amount_cents     INTEGER     NOT NULL CHECK (amount_cents > 0),
    currency         TEXT        NOT NULL DEFAULT 'USD',
    credits_mc       BIGINT      NOT NULL CHECK (credits_mc > 0),
    status           TEXT        NOT NULL DEFAULT 'open'
                     CHECK (status IN ('open', 'paid', 'failed', 'unapplied')),
    dodo_session_id  TEXT        UNIQUE,
    dodo_payment_id  TEXT        UNIQUE,
    invoice_url      TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at          TIMESTAMPTZ,
    CHECK (status <> 'paid' OR (dodo_payment_id IS NOT NULL AND paid_at IS NOT NULL))
);
CREATE INDEX credit_checkouts_account_created ON credit_checkouts (account_id, created_at DESC);
```

- **Statuses:**
  - `open`: created, not yet paid.
  - `paid`: credited.
  - `failed`: Dodo reported a failed or cancelled payment, or creating the session failed.
  - `unapplied`: paid, but the account was gone by the time the webhook arrived (§6.3).
- **"Expired":** reads report an `open` row older than 24 hours as `expired`, and nothing writes that status. A late payment on such a row is still credited.
- **Lock risk:** it's a new table, so no hot-table lock risk.
- **Ledger:** the ledger's `purchase` reason, its `external_ref` and the unique index `credit_ledger_purchase_ref_uniq` already exist (0034).
- **Applied migrations stay untouched.** Fixing a stale comment in 0034 would change its checksum, and every server would refuse to boot. The stale lines are fixed in the credits spec only (§12).

## 5. API (app service)

### 5.1 When purchasing is on

- **Turned on:** a `PurchaseConfig` exists only when all three hold:
  - the server is `managed`;
  - credits are enabled;
  - the four Dodo settings are non-empty (§9).
- **Turned off:** the checkout routes and the webhook route answer 404, and the credits view's `purchase` is `null`.
- **Startup log:** it reports `purchasing=on (live_mode)` or `purchasing=off (<reason>)`, next to the existing `hosting_mode=… credits=… signup=…` line.

### 5.2 `POST /v1/orgs/{slug}/credits/checkouts`

- **Auth:** a new extractor, `InteractiveUser`. It runs `AuthUser`, then refuses API keys and delegated JWTs (those with a `minted_jti`, from OAuth or device pairing) with 403. That's the same test `AdminUser` applies, without the admin requirement.
- **Membership:** any member. Non-members get 404, like every `/v1/orgs/{slug}` route.
- **Body:** `{ "amount_cents": <integer> }`. Out of bounds gets 400 `invalid_request`, naming the bounds.
- **Throttle:** at most 10 checkouts created per account per rolling hour, counted in the table. Over it gets 429 `too_many_checkouts`.
- **Steps:**
  1. Insert an `open` row with the computed `credits_mc` and the buyer's id and email.
  2. Create the Dodo session (§7).
  3. Store `dodo_session_id`.
  4. Answer 201 `{checkout_id, checkout_url, credits_mc}`.
- **If Dodo fails** (an error, or no answer within 15 s): the row becomes `failed`, and the answer is 502 `payment_provider_error`.

### 5.3 `GET /v1/orgs/{slug}/credits/checkouts/{id}`

- **Who:** any member (`AuthUser`).
- **Returns:** `{id, status, amount_cents, credits_mc, created_at, paid_at}`, where `status` includes the derived `expired`.
- **Used by:** the buy card, while it waits for the credits.

### 5.4 The credits view gains two fields

`GET /v1/orgs/{slug}/credits` and the admin `GET /v1/admin/accounts/{id}/credits` both return `CreditsView`. It gains:
- `purchase`: `{min_cents, max_cents, credits_per_usd, mode: "test" | "live"}`, or `null` when purchasing is off.
- `payments`: the account's last 50 checkouts, newest first. Each has `{id, created_at, amount_cents, currency, credits_mc, status, buyer_email, invoice_url, paid_at}`.

The admin console uses the same view, so account pages there show payments with no extra API.

### 5.5 `POST /v1/webhooks/dodo`

- It sits outside the auth middleware, with a 256 KiB body limit.
- It's reached at `https://app.chakramcp.com/v1/webhooks/dodo`. Caddy already forwards everything on the app domain to `:8080`.

## 6. Webhook processing

### 6.1 Verification (Standard Webhooks, which Dodo uses)

1. **Read the input:** the raw body and the `webhook-id`, `webhook-timestamp` and `webhook-signature` headers. A missing header gets 401.
2. **Check the timestamp:** more than 5 minutes from now gets 401.
3. **Compute the signature:** `base64(HMAC-SHA256(key, "{id}.{timestamp}.{body}"))`, where `key` is `DODO_PAYMENTS_WEBHOOK_KEY` with its `whsec_` prefix removed, then base64-decoded.
4. **Compare:** constant-time, against every space-separated `v1,<signature>` in the header; several signatures allow key rotation. No match gets 401.

Rejections are counted and logged at warn without the body. Only then is the JSON parsed. Unknown fields are ignored.

### 6.2 Events

- **`payment.succeeded`:** read `data.payment_id`, `data.checkout_session_id`, `data.invoice_url` and, for the ledger's metadata, `data.total_amount`, `data.tax` and `data.currency`. Then (§6.3) in one transaction:
  1. `SELECT … FROM credit_checkouts WHERE dodo_session_id = $session FOR UPDATE`.
     - **No row** (a static payment link, another integration, or no session at all): nothing is credited. Log at ERROR and answer 200.
     - **Already `paid`:** a retry or a duplicate delivery. Answer 200.
  2. **If the account is gone:** set `status = 'unapplied'` with the payment id, log at ERROR and answer 200.
  3. **Credit it:**
     - Upsert the wallet: `balance_mc += credits_mc`. A new wallet gets its first monthly grant from the worker, as today.
     - Insert the ledger row: `reason = 'purchase'`, `external_ref = payment_id`, `balance_after_mc` from the wallet's `RETURNING`, and metadata `{checkout_id, amount_cents, currency, buyer: {user_id, email}, dodo: {total_amount, tax, currency}}`.
     - Use a plain `INSERT`. An `ON CONFLICT DO NOTHING` next to a wallet CTE would still run the CTE and double-credit.
  4. Set `status = 'paid'`, with `dodo_payment_id`, `invoice_url` and `paid_at`.
  5. Commit and answer 200.
- **`payment.failed`, `payment.cancelled`:** an `open` checkout with that session becomes `failed`. Answer 200.
- **`payment.processing`:** ignored. Answer 200.
- **`refund.*`, `dispute.*`:** log at warn with the payment id, for a manual `adjustment`. Answer 200.
- **Anything else:** ignored. Answer 200.

### 6.3 Why this is safe

- **Matching uses Dodo's `checkout_session_id`, never metadata.** Dodo's shared payment links let anyone pay any amount above the product minimum, possibly with metadata of their choosing. So a payment counts only if Dodo's session id is one of ours.
  - Dodo fixes the amount of sessions we create, so a match means the agreed price was paid.
  - `metadata.checkout_id` is only cross-checked; a mismatch is logged.
- **Exactly once:**
  - The row lock plus the `paid` check makes retries and concurrent deliveries no-ops.
  - `credit_ledger_purchase_ref_uniq` is the backstop. If it ever fires (a unique violation, 23505), the whole transaction rolls back, wallet included. That's logged at ERROR and answered 200.
- **Failures:** any other error answers 500, and Dodo retries with backoff for up to 10 hours.
- **The invariant holds:** balance = Σ ledger − Σ charges, because the wallet change and its ledger row are one transaction.
- **The relay:** it picks up the new balance at its next switch refresh (≤ 5 s). **Nothing on the invocation path changes.**

## 7. The Dodo module (`backend/app/src/dodo.rs`)

- **`DodoClient`** creates sessions.
  - **Request:** `POST {base}/checkouts` with `Authorization: Bearer <api key>`. The base is `https://test.dodopayments.com` or `https://live.dodopayments.com`, chosen by the environment setting. The body:
    ```json
    {
      "product_cart": [{ "product_id": "<DODO_PAYMENTS_PRODUCT_ID>", "quantity": 1, "amount": 1000 }],
      "customer": { "email": "<buyer email>", "name": "<buyer display name>" },
      "return_url": "<frontend base>/app/credits?account=<slug>&checkout=<id>",
      "metadata": { "checkout_id": "<id>", "account": "<slug>" }
    }
    ```
  - **Response:** `{session_id, checkout_url}`.
  - **HTTP client:** `reqwest`, already a workspace dependency (the relay uses it, with rustls). The app crate adds it, with a 15 s timeout.
  - **No SDK:** Dodo's Rust SDK isn't worth a dependency for one call and one signature check.
- **`verify_webhook(headers, body, key)`** (§6.1) and **`parse_event(body)`.** The parser returns a typed event for the types in §6.2, and `Ignored` for anything else.
- **Secrets stay out of logs.** The API key and webhook key live in a type whose `Debug` prints `***`.

## 8. Web UI

### 8.1 Navigation and dashboard

- **Desktop nav:** gains **Usage** and **Credits** after Inbox.
- **Mobile bottom bar:** gains **Credits** as a sixth tab.
- **Elsewhere:** both stay in the user menu and the command palette.
- **Dashboard:** a **Credits** card comes first in the stat row, for the personal account:
  - the balance and the calls it covers, e.g. "4,870 credits · ~48,700 calls";
  - the status;
  - a **Buy credits** link to `/app/credits`.

### 8.2 `/app/credits`

This is one account at a time, with the existing Personal/org switcher. Top to bottom:
1. **Header:** "Each accepted call costs 0.1 credit. $1 buys 1,000 credits. Every account gets 5,000 free each month, and unused credits roll over." The "coming soon" line goes, and the numbers come from the view.
2. **Balance card:** the balance and "≈ N calls", and the status (Active / Out of credits / Unlimited / Off). Also the monthly free grant, the next grant date, this month's spend and the rate limit.
3. **Buy credits card,** shown when `purchase` isn't `null`:
   - A dollar input and a live line, e.g. "$10 → 10,000 credits (100,000 calls)", computed from `purchase.credits_per_usd` and the per-call cost.
   - **Buy** creates the checkout (§5.2) and opens Dodo's overlay.
   - When payment completes, Dodo sends the browser to the `return_url`: this page with `?checkout=<id>`.
   - The card then shows "Payment received, adding credits…" and polls §5.3 every 2 s for up to 2 minutes.
     - `paid`: "Added 10,000 credits", and the page data refreshes.
     - `failed`: an error with a retry.
     - Still `open` after 2 minutes: "We'll add the credits as soon as Dodo confirms the payment."
   - The `payment_id` and `status` that Dodo appends to the return URL are ignored. Anyone could forge them; only §5.3 counts.
4. **Payments:** this account's checkouts, newest first. Each shows the date, amount, credits, who bought, a status (Pending / Paid / Failed / Expired / Not applied) and a **Receipt** link to Dodo's `invoice_url` when there is one.
5. **Last 30 days:** daily spend, as now.
6. **Credit history:** the existing ledger list. Purchases read "Credits purchased · $10 by <email>".

### 8.3 The overlay

- **The SDK:** `dodopayments-checkout` from npm, bundled into the page.
  - There's no third-party script tag, and the CSP needs no change: it sets no `script-src` or `frame-src`, and the overlay is an iframe.
- **Opening it:**
  - `DodoPayments.Initialize({ mode: purchase.mode, onEvent })` runs once.
  - `DodoPayments.Checkout.open({ checkoutUrl })` runs per purchase.
  - Dodo's own redirect to the `return_url` is kept. That's simpler than `manualRedirect`, and it covers payment methods that redirect the whole page to a bank.
- **Closed without paying** (`checkout.closed`): the card goes back to its idle state. The checkout stays `open`, then shows as Expired.

### 8.4 Elsewhere

- **The usage page:** "This is the substrate future billing draws from" becomes "Each accepted invocation spends 0.1 credit (see Credits)", with a link.
- **Credits off** (`enabled: false`, e.g. self-hosted):
  - The credits page says so, and the buy card is hidden.
  - The TypeScript `CreditsView` gains `enabled`, and `CreditStatus` gains `"off"`. That fixes today's empty status pill.
- **The admin console's account page** shows the Payments section from the same view.

## 9. Configuration

| Setting (env only, chakramcp.com) | Meaning | Default |
|---|---|---|
| `DODO_PAYMENTS_API_KEY` | Dodo API key (secret) | unset |
| `DODO_PAYMENTS_WEBHOOK_KEY` | Webhook signing key, `whsec_…` (secret) | unset |
| `DODO_PAYMENTS_PRODUCT_ID` | The pay-what-you-want product, `pdt_…` | unset |
| `DODO_PAYMENTS_ENVIRONMENT` | `test_mode` or `live_mode` | unset (required) |
| `CREDITS_PURCHASE_PER_USD` | Credits per dollar | `1000` |
| `CREDITS_PURCHASE_MIN_CENTS` | Smallest purchase | `100` |
| `CREDITS_PURCHASE_MAX_CENTS` | Largest purchase | `500000` |

- **How they're read:**
  - They're read like `UPSERT_SHARED_SECRET`, in both `app/src/main.rs` and `server/src/main.rs`, and kept on `AppState`.
  - An empty value counts as unset.
  - Bad numbers stop the server at startup, the way `CreditsConfig` does.
- **Compose** passes them as `${VAR:-}`. Older images ignore variables they don't know, so that's safe for them.
- **The chart** doesn't need them; self-hosted servers don't buy.
- **The Dodo product:**
  - one-time, pay-what-you-want, minimum $1.00 USD, a SaaS tax category;
  - Dodo's own Credits, Entitlements and Metadata sections stay empty;
  - a test-mode copy and a live-mode copy.
- **The webhook endpoint** subscribes to `payment.*`, `refund.*` and `dispute.*`.

## 10. Observability

- **Counters:**
  - `chakramcp_credit_purchases_total{status}`, where status is `paid`, `failed`, `unapplied` or `unmatched`;
  - `chakramcp_dodo_webhook_rejected_total{reason}`, where reason is `missing_header`, `stale`, `bad_signature` or `bad_json`.
  - Both go in the metric catalogue in `telemetry.rs` (`names`).
- **Log lines** carry the payment, checkout and account ids. Card data never reaches us.
- **Alerts:** unmatched and unapplied payments log at ERROR, so the existing error-log alert reaches the operator. Refunds and disputes log at warn.

## 11. Testing

- **Unit:**
  - Webhook verification against the Standard Webhooks reference test vector, plus: a tampered body, a stale or future timestamp, a missing header, several signatures where only the second matches (rotation), and a key without `whsec_`.
  - `credits_mc` math at the bounds, and amount validation.
  - Event parsing: known and unknown types, extra fields.
  - Config: all four Dodo settings or nothing; empty means unset; bad numbers fail.
- **Database (`#[sqlx::test]`):**
  - Creating a checkout, against a fake Dodo, an axum server on `127.0.0.1:0` that records the request: the row, the session id and the response shape.
  - Dodo down: the row is `failed`, and the answer is 502.
  - Auth:
    - an API key, an OAuth token or a pairing token gets 403;
    - a non-member gets 404;
    - a self-hosted or credits-off server gets 404.
  - The throttle.
  - Webhooks:
    - credited once under duplicate delivery and under concurrent delivery (modelled on the existing race test, `handlers/credits.rs`);
    - an unknown session;
    - a deleted account becomes `unapplied`;
    - failed and cancelled;
    - the reconciliation invariant (balance = Σ ledger − Σ charges);
    - the view's `payments` and `purchase`, and the derived `expired`.
- **Frontend:** `pnpm test` units for the amount parsing and the credits preview.
- **End to end, before going live:** a local stack in Dodo test mode, with Dodo's CLI forwarding test webhooks to localhost. The test keys come from a gitignored file the user creates; the session never reads or prints them.

## 12. Rollout

1. **PR-1, backend:** this spec and the plan, migration 0038, `dodo.rs`, the endpoints, the webhook, the settings, the metrics, the new free-grant default, and tests. It's inert until the Dodo settings exist.
2. **PR-2, frontend:** §8. The buy card stays hidden while `purchase` is `null`.
3. **PR-3, docs:**
   - INSTALL.md and `infra/.env.example` get the Dodo settings, marked chakramcp.com only, and the new grant.
   - The website's credits copy changes.
   - In the credits spec:
     - P4 is marked done.
     - The P4 bullet stops claiming it "introduces `HOSTING_MODE`".
     - The top-up note stops promising an "in-process refresh" that doesn't exist.
     - The "0035 drops plans" note (it's 0036) is corrected in the spec only.
4. **Test mode, locally,** end to end (§11).
5. **Go-live.** The user, since this involves secrets and Dodo's dashboard:
   - creates the live product and the live webhook endpoint;
   - sets the four `DODO_PAYMENTS_*` values in the VM's `.env`;
   - runs `docker compose … up -d`.
   - CD's managed guard is unaffected.
6. **Checks after deploy:**
   - Any wallet granted under the old default gets a top-up (§3.2).
   - The startup log says `purchasing=on (live_mode)`.
   - The user makes one real $1 purchase. Then check its ledger row, the balance, the Payments entry and the receipt link, and that the counter moved.

## 13. Risks

- **A misconfigured webhook:** payments go uncredited.
  - It shows in Dodo's delivery log and in our rejection counter.
  - Dodo retries for 10 hours, so fixing the key in time recovers them. After that, resend from Dodo's dashboard, or make an operator grant.
- **Dodo outage:** checkouts fail with a clear error. Invocations are unaffected.
- **Chargebacks and fraud:** Dodo handles disputes as merchant of record. We log them for a manual `adjustment`. A negative balance blocks the account, as it does today.
- **Price or limit changes:** they affect only new checkouts.
- **Static payment links:** never credited automatically. They're logged as unmatched for a manual decision.
- **Personal data:** `credit_checkouts.buyer_email` is visible to the account's members, like other membership data. Card details never touch our servers.
