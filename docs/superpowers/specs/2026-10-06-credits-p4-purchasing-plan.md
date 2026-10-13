# Credits P4: implementation plan

Derived from `2026-10-06-credits-p4-purchasing-design.md` (approved 2026-10-06); § numbers refer to that spec.

The work ships as three PRs, then an end-to-end run in Dodo's test mode, then go-live. Only PR-1 changes anything users see before go-live: it raises the monthly free grant to 5,000 credits. Purchasing itself stays off until the Dodo settings exist on the VM.

**Conventions**
- **Ship flow:** branch → PR → green CI → squash merge → check *that commit's* CD run (`gh run list --commit <sha>`). Commits end with the Co-Authored-By trailer, and PR bodies end with the Claude Code footer.
- **Secrets:** never print one.
  - Read the VM's `.env` with `grep`/`cut` and never `source` it.
  - Never read the repo-root `.env.local` (`DODOPAYMENT_API_KEY`). The user sets every Dodo secret themselves: test keys in a gitignored local file, live keys on the VM.
- **sqlx cache:** new `sqlx::query!` calls need `cargo sqlx prepare --workspace -- --tests` against the local Postgres on `:5544`, with `DATABASE_URL` exported in the same shell as `git commit`. Commit `backend/.sqlx`.
- **Before pushing:**
  - `cd backend && cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
  - `cd frontend && pnpm lint` (0 warnings), `pnpm test` and `pnpm build`.
  - Workflows also pass `actionlint`.
- **Migrations are forever:** 0038 is new; no applied migration is edited.
- **Personal data:** checks against production print counts and ids, never emails.

---

## PR-1: backend (§3, §4, §5, §6, §7, §9, §10)

1. **Migration `backend/migrations/0038_credit_checkouts.sql`:** the table and index from §4.
   - No foreign keys, so no locks are taken on existing tables.
   - Add a header comment that it's credits P4, and a pointer to the spec.
2. **Purchase settings (`backend/app/src/purchases/config.rs`, new):**
   - `PurchaseConfig { client: DodoClient, product_id, mode: Test|Live, credits_per_usd, min_cents, max_cents }`.
   - `fn load(env, hosting, credits_enabled) -> PurchaseSetup`, where `enum PurchaseSetup { On(PurchaseConfig), Off { reason: String, error: bool } }`.
     - **Empty values count as unset.**
     - **The checks are §9's list:**
       - all four Dodo keys or none;
       - the environment is `test_mode` or `live_mode`;
       - `per_usd > 0`;
       - `0 < min ≤ max ≤ i32::MAX`;
       - `max × per_usd × 10` stays within `MAX_AMOUNT_MC`, with checked math.
     - **The defaults:** 1000, 100 and 500000.
     - **When it's `Off`:**
       - not `managed`, or credits off → `error: false`;
       - no Dodo keys at all → `error: false`;
       - any present-but-invalid setting → `error: true`.
   - **Unit tests** for each case, using an injected env map, as `HostingSettings::from_sources` does.
3. **State and startup:**
   - `AppState` gains `purchase: Option<Arc<PurchaseConfig>>` and `with_purchase(...)`.
   - `app/src/main.rs` and `server/src/main.rs` load it next to `with_upsert_secret`.
   - **Logging:** log `purchasing=on (live_mode)` or `purchasing=off (<reason>)`, at ERROR when `error`.
   - **The gauge:** set `chakramcp_purchase_config_error` to 1 when `error`, otherwise 0.
4. **`InteractiveUser` (`backend/app/src/auth.rs`):** `AuthUser`, then 403 for `api_key_id.is_some()` or `is_delegated_token(jti)`. It's `AdminUser` without the admin check. `AdminUser` keeps its own code; the two share `is_delegated_token`.
5. **Dodo client and webhook (`backend/app/src/purchases/dodo.rs`, new):**
   - **`DodoClient::new(base_url, api_key)`:**
     - `base_url` is `https://test.dodopayments.com` or `https://live.dodopayments.com`; tests inject one.
     - `reqwest` with rustls and a 15 s timeout. `reqwest` joins the app crate's dependencies from the workspace.
     - The key lives in a `Secret` newtype whose `Debug` prints `***`.
   - **`create_checkout(&self, CheckoutRequest) -> Result<CreatedSession { session_id, checkout_url }>`** sends `POST /checkouts` with §7's body, including `billing_currency` and both `feature_flags`.
   - **`verify_webhook(headers, body, key: &WebhookKey, now) -> Result<(), Rejection>`:**
     - the Standard Webhooks check from §6.1;
     - `WebhookKey::parse` strips `whsec_` and base64-decodes;
     - signatures are compared in constant time with `subtle`;
     - `Rejection` carries the counter's `reason`.
   - **`parse_event(body) -> Result<Event, Rejection::BadJson>`:**
     - `Event::PaymentSucceeded(Payment)`, `PaymentFailed(Payment)`, `PaymentCancelled(Payment)` and `Ignored(String)`;
     - `refund.*` and `dispute.*` are logged at warn as `Ignored`;
     - `Payment` has `payment_id` required and everything else as `Option`.
6. **Service (`backend/app/src/purchases/mod.rs`, new):**
   - **`create_checkout(state, account_id, slug, user, amount_cents)`:**
     - validate the bounds;
     - throttle: at most 10 rows for the account in the last hour;
     - insert `open`, call Dodo, then store the session id;
     - on a Dodo error, mark the row `failed` and return 502.
   - **`checkout_status(db, account_id, id)`:** derives `expired`, and returns 404 if the checkout belongs to another account.
   - **`recent_payments(db, account_id, 50)`.**
   - **`apply_succeeded(state, payment)`:**
     - §6.2 step by step, in one transaction with `SET LOCAL lock_timeout = '5s'`;
     - it returns an outcome enum (`Paid | Duplicate | Unmatched | Unapplied(reason) | OtherProduct`), which the handler maps to logs and counters.
   - **`apply_failed(db, payment)`:** `open` → `failed`.
   - **Wallet and ledger writes** are two statements in the transaction:
     - an upsert of the wallet with `RETURNING balance_mc`;
     - a plain `INSERT` into the ledger.
     - No `ON CONFLICT` goes on the ledger insert. A 23505 rolls back, is logged at ERROR, and the webhook still answers 200.
7. **Handlers and routes (`backend/app/src/handlers/purchases.rs`, new; `backend/app/src/lib.rs`):**
   - **`POST /v1/orgs/{slug}/credits/checkouts`:** `InteractiveUser`, plus the membership check the other org routes use.
   - **`GET /v1/orgs/{slug}/credits/checkouts/{id}`:** `AuthUser` and membership.
   - **`POST /v1/webhooks/dodo`:** takes `HeaderMap` and `Bytes`, with `DefaultBodyLimit::max(256 * 1024)` on that route only.
     - **Responses:** the codes in §6.1 and §6.2. A parse failure after a good signature gets 500.
     - **Missing purchase setup:** each route answers 404 when `state.purchase` is `None`. The routes are always mounted, so tests can toggle purchasing.
8. **The credits view (`backend/app/src/credits_service.rs`):**
   - **`CreditsView` gains:**
     - `purchase: Option<PurchaseInfo { min_cents, max_cents, credits_per_usd, mode }>`, from `state.purchase`;
     - `payments: Vec<Payment>`, from `recent_payments`.
   - **`LedgerEntry` gains** `purchase: Option<{ amount_cents, currency, buyer_email }>`, read from the metadata of `purchase` rows and visible to members. `by` stays admin-only.
   - **The admin view** shows them too.
9. **The free grant (§3.2):**
   - `default_monthly_free_mc` becomes `5_000_000` in `backend/shared/src/credits.rs`. Update its default test, which asserts `100_000`.
   - Compose's `${CREDITS_DEFAULT_MONTHLY_FREE_MC:-5000000}`.
   - The commented line in `server/src/main.rs`'s `init` template.
10. **Metrics (`backend/shared/src/telemetry.rs`):**
    - **Names:** `CREDIT_PURCHASES_TOTAL`, `DODO_WEBHOOK_REJECTED_TOTAL` and `PURCHASE_CONFIG_ERROR`.
    - **`describe()`** describes them and registers every value at 0: `status` = `paid`, `failed`, `unapplied`, `unmatched`; `reason` = `missing_header`, `stale`, `bad_signature`, `bad_json`.
    - **A test** renders `/metrics` and finds all eight series at 0.
11. **Alert rules (`infra/observability/grafana/provisioning/alerting/rules.yml`):** three rules in the existing format (§10).
    - **`credit-purchase-needs-attention`:** `for: 0m`. The expression uses `increase(...[15m]) > 0`.
    - **`dodo-webhook-failures`:** `for: 0m`.
    - **`purchase-config-error`:** `for: 1m`.
    - Escape any `$` as `$$`; the provisioning file interpolates it.
    - Observability CI's config test must pass.
12. **CD check (`.github/workflows/cd.yml`):** after the managed guard, a step counts the distinct `DODO_PAYMENTS_(API_KEY|WEBHOOK_KEY|PRODUCT_ID|ENVIRONMENT)` keys with a non-empty value in `/opt/chakramcp/.env`, over SSH. It prints key names only. It fails unless the count is 0 or 4, with an `::error::` that names the fix.
13. **Compose (`infra/docker-compose.prod.yml`):**
    - Pass the four `DODO_PAYMENTS_*` and three `CREDITS_PURCHASE_*` settings as `${VAR:-}`.
    - Older images ignore them, and empty means unset in the new parser.
14. **`.gitignore`:** add `backend/.env.dodo-test`. That's where the user keeps the test-mode keys for the end-to-end run.
15. **Tests (`#[sqlx::test(migrations = "../migrations")]`):** §11's database list, in `backend/app/src/handlers/purchases.rs`.
    - **A fake Dodo:** an axum server on `127.0.0.1:0`. It records the request body, so the test can assert the feature flags and `billing_currency`, and it can return an error or hang.
    - **Signed webhook bodies:** a test helper builds them with a key and a timestamp.
    - **The concurrency test** copies `handlers/credits.rs`'s race test with two concurrent deliveries, and also checks the reconciliation invariant.
    - **The unit tests from §11 live next to their code:** the webhook vector with an injected `now`, the parsing and the config.
16. **This PR also commits** the spec and this plan, as in Phase 3.
17. **Verify locally** against `:5544`, with the server running and no Dodo keys set:
    1. The log says `purchasing=off (no Dodo settings)` at info, and `/metrics` shows the gauge at 0 and the zeroed counters.
    2. Set only `DODO_PAYMENTS_PRODUCT_ID`. The server still starts, logs `purchasing=off (… missing)` at ERROR, and the gauge reads 1.
    3. `POST /v1/webhooks/dodo` answers 404 while purchasing is off.
18. **Ship.** Merge, then:
    - check CD: the new check passes, since there are no Dodo keys yet;
    - check the relay log for `purchasing=off`, with the gauge at 0;
    - Grafana shows the three new rules, and none is firing;
    - **the grant top-up (§3.2):**
      - count wallets whose `free_grant_period` is this month and that have no override. A ledger `free_grant` of 100,000 mc identifies the old default.
      - For each, run `chakramcp-server credits grant <account> 4900 --note "P4: monthly grant raised to 5,000"`.
      - Report the count.

## PR-2: frontend (§8)

1. **API types (`frontend/src/lib/api.ts`):**
   - `CreditsView` gains `enabled`, `purchase` and `payments`.
   - `CreditStatus` gains `"off"`.
   - `CreditsLedgerEntry` gains `purchase`.
   - New `Payment` and `CheckoutStatus` types.
   - New functions: `createCheckout(token, slug, amountCents)` and `getCheckout(token, slug, id)`.
2. **Server actions (`frontend/src/app/(app)/app/credits/actions.ts`, new):** `startCheckout(slug, amountCents)` and `checkoutStatus(slug, id)`. They follow the admin `actions.ts` pattern: get the session token and return `{ok, …} | {ok: false, error}`.
3. **Amount math (`frontend/src/lib/credits-math.ts`, new):**
   - `parseDollars("10.5") → 1050`, rejecting more than two decimals, negatives and junk;
   - `creditsFor(cents, perUsd)`;
   - `callsFor(credits, costMc)`;
   - formatting;
   - plus `pnpm test` units.
4. **Buy card (`frontend/src/components/credits/BuyCredits.tsx`, new, client):**
   - **States:** idle → creating → overlay open → (redirect back) → confirming → done / failed / still pending / needs review.
   - **The overlay:** `dodopayments-checkout`.
     - `Initialize({ mode, onEvent })` runs once per mount.
     - `Checkout.open({ checkoutUrl })` runs per purchase.
     - `checkout.closed` without a redirect goes back to idle.
   - **Polling:** with `?checkout=<id>` present, poll `checkoutStatus` every 2 s for up to 2 minutes, as §8.2 describes. On `paid`, call `router.refresh()`.
   - **Dependency:** `pnpm add dodopayments-checkout` (pnpm 9.15.9), then check the lockfile and `pnpm audit --audit-level moderate`.
5. **`CreditsPanel.tsx`:**
   - the balance card with "≈ N calls" and the status, including Off;
   - a Payments section, with a status label per §8.2 and a Receipt link;
   - purchase lines in the history;
   - a credits-off message.
   - It takes an optional `buy` slot from the page, so the admin page reuses it without the buy card.
6. **The credits page (`app/(app)/app/credits/page.tsx`):**
   - the new header copy, built from the view;
   - render `BuyCredits` when `purchase` is set;
   - pass along `searchParams.checkout`.
7. **Navigation and dashboard:**
   - **`AppNav.tsx`:** add Usage and Credits.
   - **`BottomTabBar.tsx`:** add Credits, with an icon in the existing style.
   - **The dashboard (`app/(app)/app/page.tsx`):** a Credits `StatCard` comes first, for the personal account's view. Its "Buy credits" link appears only when `purchase` is set.
8. **The usage page:** the copy change in §8.4.
9. **The admin account page:** shows Payments through `CreditsPanel`.
10. **Verify:**
    - `pnpm lint`, `test` and `build`.
    - **Locally:** the frontend plus the PR-1 backend in managed mode, with purchasing off:
      - the nav, dashboard card, Payments (empty) and history render;
      - there's no buy card;
      - a self-hosted backend shows "Credits are off".
    - **On the deploy preview:** the CSP is unchanged, and the pages load without console errors.
11. **Ship.** Merge, then Netlify deploys production. Check that the nav, dashboard card and credits page render for a signed-out redirect and a public page, and that the production bundle contains the buy card's code. It stays hidden until purchasing is on.

## PR-3: docs (§12.3)

- **`docs/INSTALL.md`'s settings table:** the seven purchase settings, marked chakramcp.com only, and the grant default of 5,000 credits.
- **`docs/self-hosting/compose.md`'s credits section:** the new grant.
- **`infra/.env.example`:** the commented Dodo block in the "chakramcp.com only" group.
- **The website:**
  - the FAQ's "currently free to join";
  - the concepts page's "$0.001 per invocation", now "$0.0001";
  - any other copy that mentions the old 100 credits or the 1,000-call month. `grep` for them.
- **The credits spec (`docs/specs/2026-09-23-credit-ledger-foundation-design.md`):**
  - P4 is marked done, with a link to this spec;
  - the P4 bullet stops claiming it introduces `HOSTING_MODE`;
  - the top-up note stops promising an in-process refresh;
  - a note that 0034's "0035 drops plans" means 0036.
- **This spec's status** becomes "implemented", with the PR numbers.

## Test mode, end to end (§11, §12.4)

1. **The user** creates `backend/.env.dodo-test`, which is gitignored, containing:
   - test-mode `DODO_PAYMENTS_API_KEY` and `DODO_PAYMENTS_WEBHOOK_KEY`;
   - `DODO_PAYMENTS_PRODUCT_ID=pdt_…` (the test product);
   - `DODO_PAYMENTS_ENVIRONMENT=test_mode`.
2. **Run the stack locally.**
   - The backend runs in managed mode with credits on and that file loaded into its environment. The session never prints the file.
   - The frontend runs on `:3000` with `CAPTCHA_ENABLED=false`, as in the #371 test.
   - It uses a throwaway database and test accounts from `infra/e2e/fixtures.env`.
3. **Webhooks to localhost:** Dodo's CLI (`dodo wh listen`) forwards test webhooks to `http://localhost:8080/v1/webhooks/dodo`. Ask the user before downloading the CLI.
4. **Buy credits in the overlay with Dodo's published test cards.**
   - Confirm with the user first.
   - Buy $2.37, and check:
     - it's credited once, as 2,370 credits;
     - the ledger row and its metadata;
     - the Payments row, with a receipt link;
     - `paid` on the counter.
   - Then:
     - a declining test card, followed by a good one in the same checkout: credited once;
     - a resent event from Dodo's dashboard: a duplicate, credited no more;
     - closing the overlay without paying: back to idle, and the checkout shows as Expired after 24 hours.
5. **Clean up:** stop the servers and drop the database.

## Go-live (§12.5–6)

1. **The user:**
   - creates the live pay-what-you-want product (minimum $1, SaaS, Adaptive Currency off) and the live webhook endpoint (`https://app.chakramcp.com/v1/webhooks/dodo`, with `payment.*`, `refund.*` and `dispute.*`);
   - appends the four live `DODO_PAYMENTS_*` lines to `/opt/chakramcp/.env`, with `DODO_PAYMENTS_ENVIRONMENT=live_mode`.
2. **Apply it:** `docker compose -f docker-compose.yml up -d relay`, run by me or the user.
3. **Check:**
   - the log says `purchasing=on (live_mode)`;
   - the gauge reads 0;
   - the credits view's `purchase` is set, and the buy card appears.
4. **The user** buys **$2.37** on chakramcp.com. Check:
   - 2,370 credits added;
   - one ledger row;
   - the Payments row and its receipt;
   - `chakramcp_credit_purchases_total{status="paid"}` is 1;
   - no alert fired.
5. **The next deploy:** CD's all-or-none check passes with four keys.
