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
| Crediting | Only Dodo's signed webhook, only for a payment whose `checkout_session_id` is one of our checkouts, and only if it paid what we asked: USD, no discount, at least the amount (§6.3) |
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
  - If someone invokes before PR-1 deploys and gets the old 100, they're topped up right after that deploy (§12) with the existing `chakramcp-server credits grant`.
  - No new command is needed.
- **Per-account overrides** (`monthly_free_grant_mc`, set by an admin) keep winning, as today.

## 4. Data: `credit_checkouts` (migration 0038)

```sql
CREATE TABLE credit_checkouts (
    id               UUID        PRIMARY KEY,
    account_id       UUID        NOT NULL,      -- no FK, like credit_ledger: outlives a deleted org
    user_id          UUID        NOT NULL,      -- the buyer; no FK either (see below)
    buyer_email      TEXT        NOT NULL,      -- for the payments list, even after the user goes
    amount_cents     INTEGER     NOT NULL CHECK (amount_cents > 0),
    currency         TEXT        NOT NULL DEFAULT 'USD',
    credits_mc       BIGINT      NOT NULL CHECK (credits_mc > 0),
    status           TEXT        NOT NULL DEFAULT 'open'
                     CHECK (status IN ('open', 'paid', 'failed', 'unapplied')),
    unapplied_reason TEXT        CHECK (unapplied_reason IN ('account_gone', 'not_as_agreed')),
    dodo_session_id  TEXT        UNIQUE,
    dodo_payment_id  TEXT        UNIQUE,
    invoice_url      TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at          TIMESTAMPTZ,
    CHECK (status <> 'paid' OR (dodo_payment_id IS NOT NULL AND paid_at IS NOT NULL)),
    CHECK ((status = 'unapplied') = (unapplied_reason IS NOT NULL))
);
CREATE INDEX credit_checkouts_account_created ON credit_checkouts (account_id, created_at DESC);
```

- **Statuses:**
  - `open`: created, not yet paid.
  - `paid`: credited.
  - `failed`: Dodo reported a failed or cancelled payment, or creating the session failed. A `failed` row can still become `paid`: after a declined card, the buyer can retry in the same Dodo session.
  - `unapplied`: paid but not credited, for the operator to resolve (§6.2). `unapplied_reason` says why:
    - `account_gone`: the account was deleted before the webhook arrived;
    - `not_as_agreed`: the payment didn't match the checkout (a discount, another currency, or less than the amount).
- **"Expired":** reads report an `open` row older than 24 hours as `expired`, and nothing writes that status. A late payment on such a row is still credited.
- **No foreign keys:** without them, the migration takes no lock on `users` or `accounts`. It only creates a table and its index, so it can't queue behind traffic on a hot table.
- **Ledger:** the ledger's `purchase` reason, its `external_ref` and the unique index `credit_ledger_purchase_ref_uniq` already exist (0034).
- **Applied migrations stay untouched.** Fixing 0034's stale "0035 drops plans" comment would change its checksum, and every server would refuse to boot. The credits spec gets a note instead (§12).

## 5. API (app service)

### 5.1 When purchasing is on

- **Turned on:** a `PurchaseConfig` exists only when all three hold:
  - the server is `managed`;
  - credits are enabled;
  - the four Dodo settings are non-empty (§9).
- **Turned off:** the checkout routes and the webhook route answer 404, and the credits view's `purchase` is `null`.
- **A problem in the purchase settings** turns purchasing off, but never stops the server. The problem could be some Dodo settings set without the others, an unknown environment, or bad prices or limits (§9).
  - The startup log says what's wrong at ERROR, e.g. `purchasing=off (DODO_PAYMENTS_WEBHOOK_KEY is missing)`.
  - The same `chakramcp-server` also runs the relay, so refusing to start would take invocations down over a half-edited `.env`.
  - CD checks the VM's `.env` before deploying: all four `DODO_PAYMENTS_*` keys or none, like the existing managed guard (§12).
  - With none of them set, purchasing is simply off, and that's logged at info.
  - **A gauge makes a broken setup visible after go-live:** `chakramcp_purchase_config_error` is 1 when any `DODO_PAYMENTS_*` or `CREDITS_PURCHASE_*` setting is present but the check failed, and 0 otherwise. It's set at startup and alerted on (§10). Otherwise a setting broken by a later edit would leave the webhook answering 404 until Dodo gave up, with no alert.
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

- **Who:** any member (`AuthUser`). The checkout must belong to `{slug}`'s account; otherwise the answer is 404.
- **Returns:** `{id, status, amount_cents, credits_mc, created_at, paid_at}`, where `status` includes the derived `expired`.
- **Used by:** the buy card, while it waits for the credits.

### 5.4 The credits view gains two fields

`GET /v1/orgs/{slug}/credits` and the admin `GET /v1/admin/accounts/{id}/credits` both return `CreditsView`. It gains:
- `purchase`: `{min_cents, max_cents, credits_per_usd, mode: "test" | "live"}`, or `null` when purchasing is off.
- `payments`: the account's last 50 checkouts, newest first. Each has `{id, created_at, amount_cents, currency, credits_mc, status, buyer_email, invoice_url, paid_at}`.

Also, each `purchase` entry in the view's ledger list gains `purchase: {amount_cents, currency, buyer_email}`, read from the ledger row's metadata (§6.2). Every member sees it, unlike the admin-only `by` field. It's what the history line "Credits purchased · $10 by <email>" needs.

The admin console uses the same view, so account pages there show payments with no extra API.

### 5.5 `POST /v1/webhooks/dodo`

- The app has no auth middleware; handlers authenticate through extractors. This handler simply takes no `AuthUser`. Its signature check (§6.1) is the only way in. It has a 256 KiB body limit (`DefaultBodyLimit` on this route).
- It's reached at `https://app.chakramcp.com/v1/webhooks/dodo`. Caddy already forwards everything on the app domain to `:8080`.

## 6. Webhook processing

### 6.1 Verification (Standard Webhooks, which Dodo uses)

1. **Read the input:** the raw body and the `webhook-id`, `webhook-timestamp` and `webhook-signature` headers. A missing header gets 401.
2. **Check the timestamp:** more than 5 minutes from now gets 401.
3. **Compute the signature:** `base64(HMAC-SHA256(key, "{id}.{timestamp}.{body}"))`, where `key` is `DODO_PAYMENTS_WEBHOOK_KEY` with its `whsec_` prefix removed, then base64-decoded.
4. **Compare:** constant-time, against every space-separated `v1,<signature>` in the header; several signatures allow key rotation. No match gets 401.

Rejections are counted and logged at warn without the body. Only then is the JSON parsed:
- Unknown fields are ignored.
- Of a payment's fields, only `payment_id` is required for parsing. `checkout_session_id`, `invoice_url`, `tax`, `discounts` and `product_cart` can each be `null`. But a missing `currency` or `total_amount` fails the price check in §6.2; it never skips it.
- A signed payload that still fails to parse is logged at ERROR and answered 500, so Dodo keeps retrying while a fix ships.

### 6.2 Events

- **`payment.succeeded`:** in one transaction (`SET LOCAL lock_timeout = '5s'`, as `add_entry` does):
  1. **Find the checkout:** `SELECT … FROM credit_checkouts WHERE dodo_session_id = $session FOR UPDATE`.
     - **No session id, or no row:**
       - If `data.product_cart` lists at least one product and none of them is our credits product (`DODO_PAYMENTS_PRODUCT_ID`), it belongs to some other integration on the Dodo account. Log at info and answer 200.
       - Otherwise, including a `null` or empty cart, it's an **unmatched** payment, e.g. through a shared payment link: nothing is credited. Log at ERROR, count it (§10) and answer 200.
     - **`paid` or `unapplied` with the same `payment_id`:** a retry or duplicate delivery. Answer 200, without logging or counting again.
     - **`paid` or `unapplied` with a different `payment_id`:** a second payment on one session, which isn't credited. Log at ERROR, count it as unmatched and answer 200.
     - **`open` or `failed`:** carry on. A `failed` row is credited too: after a declined card, the buyer can retry in the same session.
  2. **Check it paid what we asked.** All three must hold:
     - `data.currency == "USD"`;
     - `data.total_amount >= amount_cents`. That's true whether tax is added on top or included. A missing value fails.
     - `data.discounts` is empty or `null`. This check stays even with the amount check, because with tax on top a discount smaller than the tax would still pass the amount test.

     Otherwise set `status = 'unapplied'` and `unapplied_reason = 'not_as_agreed'` with the payment id, log at ERROR, count it and answer 200. The operator decides between a refund and a manual grant.
     - `unapplied` rows have no "resolved" state. They keep showing "Needs review", and the operator's refund or grant shows in Dodo and in the ledger.
  3. **Check the account still exists.** If not, set `unapplied` with `account_gone`, log at ERROR, count it and answer 200.
  4. **Credit it:**
     - Upsert the wallet: `balance_mc += credits_mc`. A new wallet gets its first monthly grant from the worker a few seconds later, as today.
     - Insert the ledger row: `reason = 'purchase'`, `external_ref = payment_id`, `balance_after_mc` from the wallet's `RETURNING`, and metadata `{checkout_id, amount_cents, currency, buyer: {user_id, email}, dodo: {total_amount, tax, currency}}`.
     - Use a plain `INSERT`. An `ON CONFLICT DO NOTHING` next to a wallet CTE would still run the CTE and double-credit.
  5. Set `status = 'paid'`, with `dodo_payment_id`, `invoice_url` and `paid_at`.
  6. Commit, count it as paid and answer 200.
- **`payment.failed`, `payment.cancelled`:** an `open` checkout with that session becomes `failed`. Answer 200.
- **`payment.processing`:** ignored. Answer 200.
- **`refund.*`, `dispute.*`:** log at warn with the payment id, for a manual `adjustment`. Answer 200.
- **Anything else:** ignored. Answer 200.

### 6.3 Why this is safe

- **Matching uses Dodo's `checkout_session_id`, never metadata.** Dodo's shared payment links let anyone pay any amount above the product minimum, possibly with metadata of their choosing. So a payment counts only if Dodo's session id is one of ours.
  - `metadata.checkout_id` is only cross-checked; a mismatch is logged.
- **The price is enforced twice.**
  - Our sessions set `amount` and turn off discount codes and currency selection (§7). Dodo allows both by default.
  - The webhook still checks what was actually paid (§6.2 step 2). Two cases would otherwise slip through:
    - A **fixed-price product** silently ignores `amount`. A $1 fixed product would sell any number of credits for $1.
    - A **discount** created later in Dodo's dashboard.
  - The go-live check uses an odd amount for the same reason (§12).
- **Exactly once:**
  - The row lock plus the status rules in step 1 make retries and concurrent deliveries no-ops.
  - `credit_ledger_purchase_ref_uniq` is the backstop. If it ever fires (a unique violation, 23505), the whole transaction rolls back, wallet included. That's logged at ERROR and answered 200.
- **Failures:** any other error answers 500, and Dodo retries. Delivery makes 8 attempts: immediately, then after 5 s, 5 min, 30 min, 2 h, 5 h, 10 h and 10 h, so about 27 hours in all.
- **The invariant holds:** balance = Σ ledger − Σ charges, because the wallet change and its ledger row are one transaction.
- **The relay:** it picks up the new balance at its next switch refresh (≤ 5 s). **Nothing on the invocation path changes.**

## 7. The Dodo module (`backend/app/src/dodo.rs`)

- **`DodoClient`** creates sessions.
  - **Request:** `POST {base}/checkouts` with `Authorization: Bearer <api key>`. The base is `https://test.dodopayments.com` or `https://live.dodopayments.com`, chosen by the environment setting. The body:
    ```json
    {
      "product_cart": [{ "product_id": "<DODO_PAYMENTS_PRODUCT_ID>", "quantity": 1, "amount": 1000 }],
      "customer": { "email": "<buyer email>", "name": "<buyer display name>" },
      "billing_currency": "USD",
      "feature_flags": { "allow_discount_code": false, "allow_currency_selection": false },
      "return_url": "<frontend base>/app/credits?account=<slug>&checkout=<id>",
      "metadata": { "checkout_id": "<id>", "account": "<slug>" }
    }
    ```
  - **Response:** `{session_id, checkout_url}`.
  - **HTTP client:** `reqwest`, already a workspace dependency (the relay uses it, with rustls). The app crate adds it, with a 15 s timeout.
  - **For tests:** the client takes its base URL as a value, so tests can point it at a fake Dodo.
  - **No SDK:** Dodo's Rust SDK isn't worth a dependency for one call and one signature check.
- **`verify_webhook(headers, body, key, now)`** (§6.1) and **`parse_event(body)`.**
  - `now` is passed in so tests can replay the Standard Webhooks reference vector, which is dated 2021.
  - The parser returns a typed event for the types in §6.2, and `Ignored` for anything else.
- **Secrets stay out of logs.** The API key and webhook key live in a type whose `Debug` prints `***`.

## 8. Web UI

### 8.1 Navigation and dashboard

- **Desktop nav:** gains **Usage** and **Credits** after Inbox.
- **Mobile bottom bar:** gains **Credits** as a sixth tab.
- **Elsewhere:** both stay in the user menu and the command palette.
- **Dashboard:** a **Credits** card comes first in the stat row, for the personal account:
  - the balance and the calls it covers, e.g. "4,870 credits · ~48,700 calls";
  - the status;
  - a **Buy credits** link to `/app/credits`, only when `purchase` isn't `null`; otherwise a plain **Credits** link.

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
     - `failed` doesn't stop the polling. A declined first attempt can be followed by a successful one in the same session, and a buyer offered a retry too early might pay twice.
     - Still `open` after 2 minutes: "We'll add the credits as soon as Dodo confirms the payment. Check Payments below."
     - Still `failed` after 2 minutes: "The payment didn't go through. You can try again; if Dodo did charge you, the credits will still arrive."
     - `unapplied`: "The payment needs a manual check. We'll sort it out."
   - On a first purchase for an account with no wallet yet, the purchased credits show first. The monthly free grant follows on the worker's next pass, a few seconds later.
   - The `payment_id` and `status` that Dodo appends to the return URL are ignored. Anyone could forge them; only §5.3 counts.
4. **Payments:** this account's checkouts, newest first. Each shows the date, amount, credits, who bought, a status (Pending / Paid / Failed / Expired / Needs review) and a **Receipt** link to Dodo's `invoice_url` when there is one.
5. **Last 30 days:** daily spend, as now.
6. **Credit history:** the existing ledger list. Purchases read "Credits purchased · $10 by <email>", from the ledger entries' new `purchase` field (§5.4).

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
- **Validated at startup.** Any failure turns purchasing off with an ERROR log naming the problem (§5.1). It never stops the server. The checks:
  - all four Dodo settings, or none;
  - the environment is `test_mode` or `live_mode`;
  - `per_usd > 0`;
  - `0 < min ≤ max`;
  - `max` fits the `amount_cents INTEGER` column;
  - `max × per_usd × 10`, computed with overflow checks, stays within `MAX_AMOUNT_MC`.
- **The `init` template:** its commented `credits_default_monthly_free_mc = 100000` line (`backend/server/src/main.rs`) moves to the new default.
- **Compose** passes them as `${VAR:-}`. Older images ignore variables they don't know, so that's safe for them.
- **The chart** doesn't need them; self-hosted servers don't buy.
- **The Dodo product:**
  - one-time, pay-what-you-want, minimum $1.00 USD, a SaaS tax category;
  - Dodo's own Credits, Entitlements and Metadata sections stay empty;
  - a test-mode copy and a live-mode copy.
- **Adaptive Currency stays off** in the Dodo dashboard. Sessions also pass `billing_currency: "USD"`. Otherwise checkout could default to the buyer's local currency, and with currency selection off they couldn't switch back. Every non-US purchase would then be held as `not_as_agreed`.
- **The webhook endpoint** subscribes to `payment.*`, `refund.*` and `dispute.*`.

## 10. Observability

- **Counters:**
  - `chakramcp_credit_purchases_total{status}`, where status is `paid`, `failed`, `unapplied` or `unmatched`;
  - `chakramcp_dodo_webhook_rejected_total{reason}`, where reason is `missing_header`, `stale`, `bad_signature` or `bad_json`.
  - A gauge, `chakramcp_purchase_config_error`, set to 0 or 1 at startup (§5.1).
  - All three go in the metric catalogue in `telemetry.rs` (`names`).
  - **At startup, every `status` and `reason` value is registered at 0** (`counter!(…).increment(0)`, next to `describe()`).
    - Without that, a series only appears at its first event, already at 1. Prometheus's `increase()` reads such a series as 0.
    - Every deploy resets the counters, so a lone stuck payment would never alert.
    - A test checks that `/metrics` lists them all at 0.
- **Log lines** carry the payment, checkout and account ids. Card data never reaches us.
- **Alerts:** the existing `error-log-spike` alert only fires above 20 ERROR lines in 5 minutes, so it would never notice one stuck payment. PR-1 adds three rules to `infra/observability/grafana/provisioning/alerting/rules.yml`:
  - **Payment needs attention:** `increase(chakramcp_credit_purchases_total{status=~"unmatched|unapplied"}[15m]) > 0`. A customer paid and wasn't credited.
  - **Webhook signature or payload failures:** `increase(chakramcp_dodo_webhook_rejected_total{reason=~"bad_signature|bad_json"}[15m]) > 0`.
    - `bad_signature` usually means a wrong `DODO_PAYMENTS_WEBHOOK_KEY`, and every payment would go uncredited.
    - `bad_json` is a signed payload our parser rejects, e.g. after a change on Dodo's side. Every payment would fail with a 500 until it's fixed.
    - The rule leaves out `missing_header` and `stale`. Scanners probing the URL send no valid signature headers, so they don't page anyone.
  - **Purchase settings broken:** `chakramcp_purchase_config_error > 0`. Purchasing is off because of a bad setting, so webhooks answer 404 and open checkouts would never be credited.
- **Refunds and disputes** log at warn, for the manual adjustment.

## 11. Testing

- **Unit:**
  - Webhook verification against the Standard Webhooks reference test vector, plus: a tampered body, a stale or future timestamp, a missing header, several signatures where only the second matches (rotation), and a key without `whsec_`.
  - `credits_mc` math at the bounds, and amount validation.
  - Event parsing: known and unknown types, extra fields.
  - Config: all four Dodo settings or nothing; empty means unset; each bad value turns purchasing off with the reason, and the server still starts.
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
    - **failed, then succeeded** on the same session: credited once;
    - **already paid, then a different `payment_id` for the session:** not credited, ERROR and counted;
    - **not as agreed** (a discount, a non-USD currency, or `total_amount` below the amount): `unapplied`, nothing credited;
    - an unknown session for the credits product (unmatched, counted), and one for another product (ignored, not counted);
    - a `null` or empty cart with no matching session: unmatched, counted;
    - a missing `currency` or `total_amount`: `unapplied` with `not_as_agreed`;
    - a second payment on an `unapplied` session: not credited, ERROR and counted;
    - a deleted account becomes `unapplied` with `account_gone`;
    - cancelled; a signed payload that doesn't parse (500);
    - the reconciliation invariant (balance = Σ ledger − Σ charges);
    - the view's `payments`, `purchase` and the ledger entries' `purchase` field, and the derived `expired`;
    - `GET …/checkouts/{id}` for another account's checkout (404).
  - Config: purchasing is off when only some Dodo settings are set, and for each bounds failure, with `chakramcp_purchase_config_error` at 1. It's 0 when nothing is set and when everything is valid. Every status and reason value is on `/metrics` at 0 from startup.
- **Frontend:** `pnpm test` units for the amount parsing and the credits preview.
- **End to end, before going live:** a local stack in Dodo test mode, with Dodo's CLI forwarding test webhooks to localhost. The test keys come from a gitignored file the user creates; the session never reads or prints them.

## 12. Rollout

1. **PR-1, backend:**
   - this spec and the plan;
   - migration 0038, `dodo.rs`, the endpoints, the webhook, the settings and the metrics;
   - the three alert rules, the zero-registered counters and the config gauge (§10);
   - a CD check, next to the managed guard: the VM's `.env` must have all four `DODO_PAYMENTS_*` keys or none (§5.1). A key with an empty value counts as unset, as it does for the server;
   - the new free-grant default, including the `init` template;
   - tests.
   - Purchasing is inert until the Dodo settings exist, but **the new grant default takes effect when PR-1 deploys**. Right after that deploy, any wallet granted under the old default gets topped up (§3.2).
2. **PR-2, frontend:** §8. The buy card stays hidden while `purchase` is `null`.
3. **PR-3, docs:**
   - INSTALL.md, `compose.md` and `infra/.env.example` get the Dodo settings, marked chakramcp.com only, and the new grant.
   - The website's credits and pricing copy changes, including:
     - the FAQ's "currently free to join" answer;
     - the concepts page's "$0.001 per invocation" (now $0.0001).
   - In the credits spec:
     - P4 is marked done.
     - The P4 bullet stops claiming it "introduces `HOSTING_MODE`".
     - The top-up note stops promising an "in-process refresh" that doesn't exist.
     - A note records that migration 0034's comment "0035 drops plans" means 0036. The migration itself can't change (§4).
4. **Test mode, locally,** end to end (§11).
5. **Go-live.** The user, since this involves secrets and Dodo's dashboard:
   - creates the live product and the live webhook endpoint;
   - sets the four `DODO_PAYMENTS_*` values in the VM's `.env`;
   - runs `docker compose … up -d`.
   - CD's managed guard is unaffected.
6. **Checks after go-live:**
   - The startup log says `purchasing=on (live_mode)`.
   - The user makes one real purchase of an **odd amount**, e.g. $2.37 → 2,370 credits. A fixed-price product, which ignores our amount, would show up here; a $1 test wouldn't catch it.
   - Then check its ledger row, the balance, the Payments entry and the receipt link, and that `chakramcp_credit_purchases_total{status="paid"}` moved.

## 13. Risks

- **A misconfigured webhook:** payments go uncredited.
  - It shows in Dodo's delivery log, and the signature alert fires (§10).
  - Dodo retries for about 27 hours, so fixing the key in time recovers them. After that, resend from Dodo's dashboard, or make an operator grant.
- **Farming free credits:** the free grant is now 50 times bigger, creating an org is free, and unused credits roll over with no cap. That makes creating extra orgs to collect free credits much more attractive.
  - Not addressed here.
  - If it shows up in the admin console, the credits spec's planned free-balance cap, or a limit on orgs per user, would close it.
- **Dodo outage:** checkouts fail with a clear error. Invocations are unaffected.
- **Chargebacks and fraud:** Dodo handles disputes as merchant of record. We log them for a manual `adjustment`. A negative balance blocks the account, as it does today.
- **Price or limit changes:** they affect only new checkouts.
- **Static payment links:** never credited automatically. They're logged as unmatched for a manual decision.
- **Personal data:** `credit_checkouts.buyer_email` is visible to the account's members, like other membership data. Card details never touch our servers.
