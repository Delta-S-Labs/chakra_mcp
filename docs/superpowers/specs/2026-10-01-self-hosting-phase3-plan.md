# Self-hosting, Phase 3: implementation plan

Derived from `2026-10-01-self-hosting-phase3-design.md` (approved 2026-10-01); § numbers refer to that spec. The work ships as four PRs, then a release. Each PR leaves production unchanged, except PR-3, which adds the failed sign-in limit and the MCP discovery fix everywhere.

**Conventions**
- **Ship flow:** branch → PR → green CI → squash merge → check *that commit's* CD run (`gh run list --commit <sha>`). Commits end with the Co-Authored-By trailer, and PR bodies end with the Claude Code footer.
- **Secrets:** never print one. Read the VM's `.env` with `grep`/`cut`, never `source` it, and edit it with the backup-and-append pattern.
- **sqlx cache:** new `sqlx::query!` calls need `cargo sqlx prepare --workspace -- --tests` against the local Postgres on `:5544`, with `DATABASE_URL` exported in the same shell as `git commit`. Commit `backend/.sqlx`.
- **Before pushing:** `cd backend && cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Workflows also pass `actionlint` and shell scripts pass `shellcheck`.
- **Local Docker is colima.** Pulls use a temporary `DOCKER_CONFIG`, and bind mounts only work under `$HOME`.
- **Personal data:** nothing from production's users table goes into chat or logs. Checks print counts, not emails.

---

## PR-1: operator commands (§4)

1. **Account creation (`backend/app/src/accounts.rs`, new):**
   - `create_user(tx, NewUser { email, name, password, is_admin }) -> ApiResult<CreatedUser { user_id, account_id, account_slug }>`.
     - It runs `handlers::auth::signup`'s validation and transaction: trim and lowercase the email, require `@`, a password of 8–200 characters and a name, return 409 on a duplicate, hash with Argon2, then create the individual account, the owner membership and the personal slug.
     - The caller decides `is_admin`.
   - **Moves here:** `hash_password`, `verify_password`, and the slug helper. `handlers::users`'s copy of the slug helper (`personal_account_slug`) uses it.
   - **`signup`** calls it with today's `ADMIN_EMAIL` rule, then mints the JWT. Responses don't change.
2. **Credits service (`backend/app/src/credits_service.rs`, new):**
   - `enum Actor { Admin { user_id, email }, Operator }`. Ledger metadata becomes `{"admin": {…}}` or `{"operator": "cli"}`.
   - **Moved from `handlers/credits.rs`**, with their bounds, validation and SQL unchanged:
     - `add_entry(db, account_id, kind, amount_mc, note, actor)`
     - `update_settings(db, account_id, changes, note, actor)`
     - `load_view(db, account_id, cfg)`
   - `ledger_entry` shows operator rows as written by `operator (CLI)`.
   - The handlers become thin wrappers, and their existing tests pass unchanged.
3. **Account lookup:** `resolve_account(db, "<slug|uuid|email>")`.
   - A UUID is matched by id.
   - A value containing `@` resolves to that user's personal account: the individual account the user owns.
   - Anything else is matched by slug.
   - When nothing matches, it returns a plain error.
4. **Subcommands (`backend/server/src/main.rs`, plus `backend/server/src/ops/{mod,users,credits}.rs`):**
   - **`users`:** `add <email> --name <n> [--admin] [--password-stdin]`, `list [--json]` (with the personal account slug), `set-admin <email> [--off]`, and `set-password <email> [--password-stdin]`.
   - **`credits`:**
     - `show <account> [--json]`
     - `grant <account> <credits> [--note]`
     - `adjust <account> <±credits> --note`, with clap's `allow_negative_numbers`
     - `set <account> [--unlimited on|off] [--monthly-grant <credits|default>] [--rate <n|default>] [--note]`
   - **Database:** each command loads config the way `migrate` does and opens a 2-connection pool.
   - **Output:** logs go to stderr, and stdout carries only results. Give `init_tracing` a writer argument, or skip it in these commands and print errors directly.
   - **Passwords:**
     - On a terminal, use `dialoguer::Password` with confirmation. dialoguer is already in the lock through the CLI; add it to the server crate.
     - `--password-stdin` reads one line.
     - If stdin isn't a terminal and the flag is missing, refuse.
   - **Amounts:** decimal credits with up to three decimals are converted to milli-credits. Finer precision is refused.
5. **Tests:**
   - `#[sqlx::test(migrations = "../migrations")]` for:
     - `create_user`: validation, duplicates, and `is_admin` coming from the caller;
     - the credits service with both actors, checking the metadata and the operator label in the view;
     - `resolve_account` for each input form.
   - Unit tests for the amount parser and clap parsing, including `adjust -5`.
   - The existing auth and credits handler tests pass unchanged.
6. **This PR also commits** the spec and this plan, as in Phase 2.
7. **Verify locally** against `:5544`:
   1. `users add a@example.test --name A --admin --password-stdin`
   2. `users list --json`, which must be valid JSON with no log lines
   3. `credits grant a@example.test 5 --note test`
   4. `credits show a@example.test --json`
8. **Ship.** Merge; it deploys the backend with no behaviour change. Then run a read-only check on the VM: `docker exec chakramcp-relay-1 chakramcp-server users list --json`, piped to a count.

## PR-2: `HOSTING_MODE` (§3, §5, the §6.1 fallback, §8.1–8.3, §8.5)

1. **Shared settings (`backend/shared/src/hosting.rs`, new):**
   - `enum HostingMode { SelfHosted, Managed }` and `struct HostingSettings { mode, signup_enabled, credits_enabled }`.
   - A `from_sources(env, file)` constructor implements §3's parsing and precedence: non-empty env, then the file, then the mode's default. Bad values are errors that name the key.
   - Unit tests cover every case, including an empty env value falling through to the file.
2. **Config:**
   - **Frontend URL:** `SharedConfig::from_env` and the server's `load_config` follow §6.1's order, with no `localhost:3000` fallback.
   - **`ServerFile` gains** `hosting_mode`, `signup_enabled`, `credits_enabled`, `credits_default_monthly_free_mc`, `credits_cost_per_invocation_mc`, `limits_default_rate_per_min`, `limits_enforce` and `redis_url`.
   - **`CreditsConfig`:**
     - Gains `enabled`.
     - `from_sources` treats empty values as unset and accepts file values. The server main passes the file's values; the standalone mains pass none.
   - **`LIMITS_ENFORCE` and `REDIS_URL`** follow the same precedence in the server main.
   - **Startup:**
     - the §3 log line;
     - in `self_hosted` mode, the §6.1 warning when the frontend URL isn't the app URL.
   - **`init`:**
     - no `frontend_base_url`;
     - commented lines for the new keys;
     - ends by printing the next steps (`migrate`, `start`, `users add … --admin`).
3. **App:**
   - `AppState` gains `hosting`.
   - **`signup`:**
     - If sign-up is closed, it returns `ApiError::SignupDisabled`: 403 `signup_disabled`, a new variant in `shared/src/error.rs`.
     - `is_admin` is true only when the mode is `managed` and the email matches `ADMIN_EMAIL`.
   - **`upsert`:**
     - In `self_hosted` mode it never changes `is_admin`. New users get false, and existing users keep theirs.
     - Creating a new user requires sign-up to be open; otherwise it returns `SignupDisabled`.
   - **Views:** `load_view` and `/v1/admin/orgs`'s `credit_status` report `enabled` and status `off` when credits are off.
4. **Relay:**
   - `limits::enforce()` skips the credit switch when credits are off.
   - **Worker `account()`, when credits are off:**
     - It deletes queued rows in batches of 1,000 (`DELETE … WHERE invocation_id IN (SELECT … LIMIT 1000 FOR UPDATE SKIP LOCKED)`).
     - It skips grants.
     - The switch refresh doesn't change.
   - **Tests:**
     - with credits off, a blocked wallet still gets through the gate;
     - the queue drains with no `invocation_charges` rows and an untouched wallet;
     - no grants are made.
5. **Compose and `.env.example` (§8.1):**
   - Pass the new settings through with empty defaults.
   - `ADMIN_EMAIL: ${ADMIN_EMAIL:-}`.
   - Drop `GOOGLE_*`, `GITHUB_*`, `RECAPTCHA_*` and `CAPTCHA_ENABLED`.
   - Fix the Caddy-routing and `CREDITS_*` comments.
   - Regroup `.env.example`, with a "chakramcp.com only" group.
6. **Chart:**
   - `config.signupEnabled` and `config.creditsEnabled`: schema boolean or null, rendered only when set.
   - README rows, and the `config.adminEmail` note.
   - `NOTES.txt` prints the exact `kubectl exec deploy/<name> -- chakramcp-server users add …` command.
7. **Local dev and examples (§8.5):**
   - The root `.env.example`, plus `Taskfile.yml`'s `dev:backend` `env:`: `HOSTING_MODE: managed` and `FRONTEND_BASE_URL: http://localhost:3000`.
   - `frontend/e2e/README.md`.
   - Both examples' READMEs, and a `signup_disabled` hint in their `setup.py`.
8. **Single binary (§8.3):**
   - The caveats in `Formula/chakramcp-server.rb` and `packaging/server/homebrew/chakramcp-server.rb.template`.
   - `docs/INSTALL.md`'s binary section: the config path, `users add`, the new keys, and the upgrade note.
   - The full docs rewrite comes in PR-4; this PR keeps the binary docs from being wrong.
9. **CD guard (`cd.yml`):** a step between "Set up SSH" and "Sync infra config to VM". It runs `grep -qx 'HOSTING_MODE=managed' /opt/chakramcp/.env` over SSH and fails with an `::error::` that explains the fix.
10. **Verify:**
    - `cargo test --workspace` and clippy.
    - Compose config renders with the new keys unset, and with production-like values.
    - Adapt the scratchpad probe from 2026-10-01 to run the server locally in both modes:
      - `self_hosted`: sign-up returns 403, the credits view reports `off`, and the log line says so;
      - `managed`: sign-up works, `ADMIN_EMAIL` becomes admin, and status is `active`.
11. **Before merge (production):**
    1. Add `HOSTING_MODE=managed` to `/opt/chakramcp/.env`: back it up, append, and confirm with `grep -c`.
    2. Dry run on the VM. Render the new Compose file with production's `.env` in a temp dir (`docker compose config`). Diff the relay environment against the running container (`docker inspect`): only the intended additions and removals.
12. **Ship.** Merge, then:
    - CD's guard passes and the relay restarts;
    - the relay log shows `hosting_mode=managed credits=on signup=open`;
    - an invalid sign-up body still gets a 400, not `signup_disabled`;
    - the web UI loads.

## PR-3: built-in pages, sign-in limit, MCP discovery, `chakramcp api-keys` (§6, §7)

1. **Refactors (`backend/app/src/handlers/oauth.rs`):**
   - Pull these out of their handlers:
     - `client_lookup(db, client_id)`
     - `issue_code(db, user, IssueCodeParams)`, from `POST /oauth/issue-code`
     - `device_session(db, user_code)`
     - `device_approve(db, user, ApproveParams)`
     - `device_deny(db, user, user_code)`
   - Add a query for the user's agents: agents in accounts the user belongs to, with id, slug, display name and account slug.
   - The handlers become wrappers, and their existing tests pass.
2. **Sign-in limit (§6.5):**
   - Migration `0037_signin_failures.sql`.
   - `app::signin_limit::{check, record_failure, clear}`, keyed by SHA-256 of the lowercased email.
     - **`check`** returns `SigninRateLimited { retry_after_secs }` when there are at least 10 failures and the window is still open.
     - **`record_failure`** upserts the count, resetting an expired window, and deletes rows older than a day.
     - **`clear`** runs on a successful sign-in.
   - `ApiError::SigninRateLimited`: 429 `signin_rate_limited` with `Retry-After`. `IntoResponse` sets the header.
   - `POST /v1/auth/login` uses the limit in all modes.
   - **Tests:**
     - 10 failures give a 429 with `Retry-After`;
     - the right password is still refused inside the window;
     - a success clears the count;
     - unknown emails count;
     - an expired window resets.
3. **Pages (`backend/app/src/pages/`, new):**
   - **Dependencies:**
     - `askama`, with templates in `backend/app/templates/pages/`;
     - `qrcode`, for SVG;
     - `axum-extra` with `cookie`;
     - `hmac` and `sha2` (workspace) for the derived key;
     - `subtle` for constant-time comparison.
   - **`pages::router(state)`** is merged into the app router only in `self_hosted` mode.
   - **Routes:** `GET/POST /oauth/authorize`, `GET/POST /signup`, `GET/POST /app/pair`, `GET /qr`, `POST /logout`, and `GET /assets/pages.css` (via `include_str!`).
   - **Modules:**
     - `session.rs`: the derived key; minting and checking the page-session JWT (1 hour, non-admin); revocation through `revoked_tokens`; cookie attributes, with `Secure` when the app origin is https.
     - `csrf.rs`: the double-submit token and §6.4's `Origin` rule.
     - `security.rs`: the app origin; response headers, including a CSP per page with the consent page's `form-action` addition; `return_to` checks (§6.3).
     - `authorize.rs`, `signup.rs`, `pair.rs` and `qr.rs`: as §6.2 and §6.3, built on the refactored functions, `create_user` and `signin_limit`.
     - Templates: `layout.html` (the header shows the app host), `signin.html`, `consent.html`, `signup.html`, `pair.html`, `qr.html` and `error.html`.
4. **Page tests** (axum `oneshot`, `#[sqlx::test]`):
   - **Sign-in:** good and bad passwords, and the limit.
   - **CSRF:** refused for a missing token, a mismatched token, a foreign `Origin` and `Origin: null`. With no `Origin` and a valid token, the request passes.
   - **Approve:** answers 303 with `code` and `state`. Redeeming the code at `/oauth/token` with the PKCE verifier yields a token that works on `/v1/me`.
   - **Deny and bad requests:**
     - deny redirects with `access_denied`;
     - an unknown client or unregistered redirect shows the error page, with no redirect;
     - a bad challenge redirects with `invalid_request`.
   - **Agent scope:** a "selected" agent scope persists.
   - **`/signup`:** absent while closed; while open it creates the account and signs in.
   - **`return_to`:** `//x`, `/\x`, `/%09/x` and `https://x` all fall back.
   - **Pairing:** approve with an existing agent and with a new agent, and deny.
   - **Other routes:**
     - `/qr` refuses URLs on other origins;
     - logout revokes the session;
     - in `managed` mode, every page returns 404.
   - **Headers:** the CSP, `frame-ancestors`, and `Referrer-Policy: same-origin`.
5. **MCP discovery (§7, `backend/relay/src/lib.rs`):**
   - A `map_response` layer adds `WWW-Authenticate: Bearer resource_metadata="<relay>/.well-known/oauth-protected-resource"` to every 401.
   - `/.well-known/oauth-protected-resource/mcp` routes to the same handler.
   - `CorsLayer::expose_headers([WWW_AUTHENTICATE])`.
   - Tests cover the header, the route and the CORS exposure.
6. **CLI (`backend/cli/src/commands/api_keys.rs`):**
   - `api-keys create|list|revoke` as in §6.6. `--account <slug>` is resolved through `/v1/me`, and `list` takes `--json`.
   - Register the command in `main.rs`.
   - Tests cover argument parsing and the request body each option builds.
7. **Then:** `sqlx prepare`, clippy and tests.
8. **Verify locally:**
   1. Run the built server in `self_hosted` mode against a fresh Postgres.
   2. Create a test account with `users add`. Its password comes from the e2e fixture file, which only holds test values.
   3. Point the CLI at it: `networks add local …`, `networks use local`.
   4. Run `login --method browser` with `BROWSER` set to print the URL, and open that URL in the built-in browser pane. Sign in, approve, and confirm the CLI receives its token.
   5. Do the same with `chakramcp pair`.
   6. Take screenshots for the PR.
9. **Ship.** Merge; the deploy brings the sign-in limit and the MCP fix to production. After CD:
   - an unauthenticated `POST https://relay.chakramcp.com/mcp` returns 401 with `WWW-Authenticate`;
   - `/.well-known/oauth-protected-resource/mcp` returns 200;
   - `https://app.chakramcp.com/oauth/authorize` still returns 404, since `managed` mode serves no pages;
   - ask the user to sign in to the web UI once, since I don't enter passwords on production.

## PR-4: CI and docs (§8.2's `helm test`, §8.4, §9)

1. **`infra/e2e/` (new):**
   - **`browser.py`:** the `BROWSER` driver, Python standard library only. It parses forms with `html.parser`, keeps cookies, sends `Origin`, follows sign-in and consent or pairing, and logs to a file.
   - **`mcp_client.py`:** register (DCR), PKCE, the pages, the token exchange, `initialize` and `tools/list`.
   - **`run.sh`:** compose.md's steps in order (§9).
   - **`fixtures.env`:** test-only values.
2. **`.github/workflows/self-host-e2e.yml`:**
   - Runs on `ubuntu-22.04` for every PR and push to main, gated by `dorny/paths-filter`.
   - Rust toolchain, plus `Swatinem/rust-cache` with key `self-host-e2e`.
   - Builds the server and CLI, stages `dist/chakramcp-server`, and runs `docker build -f infra/Dockerfile.release -t chakramcp-server:e2e .`.
   - Adds the `/etc/hosts` entries and runs `infra/e2e/run.sh`.
   - On failure, prints the Compose logs and the driver's log.
3. **Chart:** the `/oauth/authorize` check goes into `templates/tests/`. chart-ci runs against `:edge`, which has the pages after PR-3.
4. **Docs (§8.4):**
   - **Self-hosting guides:** `compose.md` (first start, accounts, credits, the local-trial paragraph), `kubernetes.md`, `INSTALL.md`, `docs/self-hosting/README.md`.
   - **Repo docs:**
     - the root `README.md`;
     - `docs/CI-CD.md`: the `HOSTING_MODE` requirement, CD's guard and the new workflow;
     - the credit ledger spec: the checklist done.
   - **Website pages:** `/docs/self-host`, plus the two pages with the wrong variable name.
   - **The spec's status line:** implemented.
5. **Verify:**
   - Run `infra/e2e/run.sh` locally on colima, with `LOG_DRIVER=json-file` and `http://` names.
   - `actionlint` and `shellcheck`.
6. **Ship:**
   1. Merge.
   2. Watch the e2e run on main.
   3. Ask the user whether to make `self-host-e2e` a required check.

## Release 0.3.0 (§11)

1. Confirm the version with the user; 0.3.0 is proposed.
2. **Bump PR:**
   - `backend/cli/Cargo.toml` and `Cargo.lock`.
   - The README and INSTALL pointers, which the install-docs-sync hook checks.
   - Release notes covering the self-host changes:
     - sign-up closed by default;
     - admins made with `users`;
     - credits off;
     - the binary's `frontend_base_url` line.
3. Merge, tag `cli-v0.3.0` on the merge commit, and watch `cli-release.yml`.
4. **Verify:**
   - the release assets;
   - the Homebrew bot PR: check its sha256s, then admin-merge it;
   - npm;
   - `:0.3.0`, `:0.3` and `:latest` on both architectures, via `--version`;
   - chart 0.3.0;
   - production's `--version` reporting 0.3.0 after its next deploy, which needs #364.
5. Refresh `docs/NEXT_STEPS.md`.
