# Self-hosting, Phase 3: a self-hosted network that works without the web UI

**Status:** implemented: #365 (operator commands), #366 (`HOSTING_MODE`), #367 (built-in pages, sign-in limit, MCP discovery, `chakramcp api-keys`), and the CI test and docs in the PR that follows them; plan: `2026-10-01-self-hosting-phase3-plan.md`
**Date:** 2026-10-01
**Builds on:**
- `2026-09-29-self-hosting-phase2-design.md` (public image, Compose, Helm chart; released in 0.2.0).
- `docs/specs/2026-09-23-credit-ledger-foundation-design.md`. This spec completes its deferred "first-self-hoster checklist" and introduces `HOSTING_MODE` here instead of in credits P4.

## 1. Goal

Someone follows our Compose guide, Kubernetes guide or single-binary guide and ends up with a working private network, without chakramcp.com's web UI:
- an admin account they created safely;
- the CLI signed in, through the browser or device pairing;
- MCP clients connected through OAuth;
- agents that pair themselves;
- SDKs working with API keys;
- no surprise credit blocks.

A CI job follows the Compose guide on every change, so the documented path keeps working.

**What a self-hoster hits on 0.2.0** (checked against the public image on 2026-10-01):
- `chakramcp login`, MCP clients' OAuth and device pairing send the browser to `/oauth/authorize`, `/app/pair` and `/qr`. Those are web UI pages, and the server answers 404.
- Credits run with chakramcp.com's defaults: enforced, a 100-credit monthly grant, 0.1 credit per call. Every account is refused after about 1,000 calls a month, and nothing can buy more. `CREDITS_COST_PER_INVOCATION_MC` can't be 0, and `LIMITS_ENFORCE=false` also turns off rate limiting.
- Sign-up is open and emails are never verified. The first account registered with `ADMIN_EMAIL` becomes admin, so on an internet-facing server a stranger who knows or guesses that address can claim it.
- SDKs need `ck_` API keys, which only the web UI creates.
- Compose never passes `CREDITS_*` to the server, although its comment says it does. The single-binary docs give the wrong config path, and `chakramcp-server init` writes `http://localhost:3000` as the sign-in address.

**Decided with the user (2026-10-01):**

| Question | Decision |
|---|---|
| Sign-in without the web UI | Small HTML pages built into the server. Not chosen: packaging the Next.js web UI, or CLI and API keys only. |
| Self-host defaults | One setting, `HOSTING_MODE`: `self_hosted` by default, `managed` on chakramcp.com |
| Credits, sign-up and admins on self-hosted servers | Credits off and sign-up closed until the operator opens them. Admins are made with an operator command, not by signing up with `ADMIN_EMAIL`. |

**Out of scope:**
- a web dashboard for self-hosters (the CLI covers management);
- GitHub, Google or OIDC sign-in on the built-in pages;
- email verification, invites, password reset by email;
- buying credits (credits P4);
- per-IP limits;
- a CLI option to trust a private certificate authority.

## 2. Principles

- **chakramcp.com behaves as before.** There are two deliberate exceptions that apply everywhere: the failed sign-in limit (§6.5) and the MCP discovery fix (§7).
  - `HOSTING_MODE=managed` goes into production's `.env` before the change that reads it.
  - CD refuses to deploy if it's missing.
- **One implementation.** The built-in pages and the operator commands call the same Rust functions as the HTTP handlers. That logic moves out of the handlers so the paths can't drift.
- **Safe defaults.** A fresh self-hosted server has sign-up closed, credits off, and no admin until the operator makes one.
- **Every PR is safe on its own** and leaves production as it was, apart from the two exceptions above.

## 3. `HOSTING_MODE`

**Parsing:**
- Read once at startup, in the shared crate, by `chakramcp-server` and the standalone app and relay binaries.
- Values are `self_hosted` and `managed`, trimmed and case-insensitive. Unset or empty means `self_hosted`. Anything else stops startup with an error naming the key.
- `server.toml` key: `hosting_mode`. As with other keys, the environment wins.

**What it decides:**

| | `self_hosted` (default) | `managed` (chakramcp.com) |
|---|---|---|
| Credits (§5) | Off | On |
| Public sign-up (`POST /v1/auth/signup`, the `/signup` page) | Closed: 403 `signup_disabled` | Open |
| `ADMIN_EMAIL` | Plays no part. Sign-up and `/v1/users/upsert` never set the admin flag, and upsert leaves a stored flag alone. Admins come from `chakramcp-server users` (§4). | As today: sign-up with that email grants admin, and upsert sets the flag from it at every GitHub/Google sign-in |
| Built-in pages (§6) | Served | Not served. chakramcp.com uses its web UI. |
| Buying credits (credits P4, later) | Not offered | Offered |

**Overrides:**
- `CREDITS_ENABLED` and `SIGNUP_ENABLED`, with `server.toml` keys `credits_enabled` and `signup_enabled`.
- They accept `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`. Unset or empty means the mode's default; anything else stops startup.
- **Precedence**, for these and every setting this spec adds to `server.toml`:
  1. a non-empty environment value;
  2. the `server.toml` key;
  3. the default.

  An empty environment value, as Compose passes for unset `.env` keys, falls through to the file.

**Startup log:** one line, e.g. `hosting_mode=self_hosted credits=off signup=closed`.

**New accounts:** closed sign-up means no new accounts by any public path.
- **`/v1/users/upsert`** still signs in existing users. It creates a new user only while sign-up is open.
- **The operator command** (§4) always works.
- **chakramcp.com** doesn't set `SIGNUP_ENABLED`, so nothing changes there.

**Notes:**
- `/v1/users/upsert` is the web UI's GitHub/Google callback and needs `UPSERT_SHARED_SECRET`, so self-hosters normally don't have it.
- Accounts that are already admins keep the flag.
- On `managed`, upsert keeps rewriting the flag from `ADMIN_EMAIL`. A `users set-admin` (§4) there lasts only until that user's next GitHub/Google sign-in. The commands are meant for self-hosted servers.

## 4. Operator commands

These are new `chakramcp-server` subcommands. They load config like `migrate` (`DATABASE_URL` and `JWT_SECRET` required), talk to the database directly, need no sign-in, and work in every mode.

- **Compose:** `docker compose -f docker-compose.prod.yml exec relay chakramcp-server users add …`
- **Kubernetes:** `kubectl -n <ns> exec deploy/<name> -- chakramcp-server …`
- **Binary:** run it directly.
- **Passing a password on stdin:** use `exec -T` with Compose and `exec -i` with kubectl.
- **Output:** stdout carries only the command's result. Logs go to stderr, so `--json` output stays parseable even when the container sets `LOG_FORMAT=json`.

| Command | What it does |
|---|---|
| `users add <email> --name <name> [--admin] [--password-stdin]` | Creates the user, a personal account and the owner membership, as sign-up does. The admin flag comes only from `--admin`, in every mode; `ADMIN_EMAIL` plays no part. On a terminal it prompts for the password twice; otherwise `--password-stdin` is required. It validates like sign-up (an email with `@`, a password of 8–200 characters, a name) and fails if the email exists. |
| `users list [--json]` | Email, name, admin flag, personal account slug and creation time. |
| `users set-admin <email> [--off]` | Sets or clears the admin flag. Tokens carry the flag, so it takes effect at the user's next sign-in, within 24 hours. |
| `users set-password <email> [--password-stdin]` | Replaces the password hash. Existing sign-ins stay valid until they expire. |
| `credits show <account> [--json]` | `<account>` is an account slug, an account id, or a user's email, meaning that user's personal account. Shows the same data as the admin view (§5). |
| `credits grant <account> <credits> [--note <text>]` | Adds credits as an `admin_grant` ledger row. |
| `credits adjust <account> <±credits> --note <text>` | Adds an `adjustment` row; the note is required. Negative amounts such as `-5` parse as values, not flags. |
| `credits set <account> [--unlimited on\|off] [--monthly-grant <credits>\|default] [--rate <per-min>\|default] [--note <text>]` | Writes the same zero-delta settings row as the admin API. |

**Details:**
- Amounts are in credits, with up to three decimals, and are converted to milli-credits. Bounds are the admin API's.
- Ledger rows record `{"operator": "cli"}` where the admin API records `{"admin": {…}}`. The views show such rows as written by "operator (CLI)".
- **What the admin flag gives on a self-hosted server:** `/v1/admin/*`, with a token from `POST /v1/auth/login`. Page sessions are non-admin (§6.4), and `AdminUser` refuses CLI, OAuth and device tokens. Day-to-day operator work goes through these commands. The docs say so.
- **Piping a password:** it needs `docker compose exec -T`, which the docs show.

**Refactor:**
- The sign-up transaction becomes `app::accounts::create_user(…)`.
- The admin credit SQL (grant, adjustment, settings, and the view loader) becomes a credits service in the app crate. It takes a transaction and an actor: an admin user, or the operator.
- The HTTP handlers become thin wrappers. Their behaviour and responses don't change.

## 5. Credits on self-hosted servers

`CreditsConfig` gains `enabled`, read from `CREDITS_ENABLED` with the default taken from the mode.

**When credits are off:**
- **Relay gate:** it skips the credit switch. Rate limiting doesn't change: `LIMITS_ENFORCE`, `LIMITS_DEFAULT_RATE_PER_MIN` and per-account rate overrides still apply.
- **Hot path:** unchanged. It still queues the charge row inside the INSERT it already runs, so no SQL on the invocation path changes.
- **Worker:**
  - The accounting step deletes queued rows in batches instead of charging them, and makes no monthly grants.
  - The switch refresh keeps running, because rate overrides come from it.
  - The credit alerts (stale switches, queue backlog) stay meaningful.
- **Views:** these gain `"enabled": false` and report status `off`. Balances don't move.
  - the owner view (`GET /v1/orgs/{slug}/credits`);
  - the admin view;
  - `GET /v1/admin/orgs`'s `credit_status`;
  - `credits show`.
- **Turning credits on later:**
  - Nothing about wallets changes while credits are off.
  - An account that never had a wallet starts fresh: its first charge creates the wallet, and it gets the monthly grant.
  - A wallet from a time when credits were on keeps its balance. It receives the grants it missed under the existing catch-up rule, up to 12 months. The docs say so.

**When credits are on:** as today. The views report `"enabled": true` with the existing statuses.

**Not affected:** the per-capability public quota (an agent owner's monthly cap on public invocations) is separate and unchanged.

**Empty values:** `CreditsConfig::from_env` treats an empty value as unset. Compose can then pass every credit setting through, and an unset `.env` key still means the default. Today an empty value stops startup.

**`server.toml`:**
- **The keys:** each of these settings gets a key named after its env var, lowercased: `credits_default_monthly_free_mc`, `credits_cost_per_invocation_mc`, `limits_default_rate_per_min`, `limits_enforce` and `redis_url`.
- **Why:** the Homebrew service runs with no environment. Without `limits_enforce` and `redis_url`, a binary install could neither refuse calls nor rate-limit them.
- **Precedence:** as in §3.
- This completes the credit spec's "credit knobs in `server.toml`" item.

## 6. Built-in pages

### 6.1 Where and when

- **When:** they're served by the app API (port 8080) when `HOSTING_MODE=self_hosted`. In `managed` mode the routes don't exist.
- **Paths:** they're the same as the web UI's, so nothing that builds these URLs changes:
  - `/oauth/authorize`
  - `/signup`
  - `/app/pair`
  - `/qr`
  - `POST /logout`
  - a stylesheet under `/assets/`
- **Discovery URLs.** The server builds `authorization_endpoint` and the device-flow URLs from `frontend_base_url`.
  - **New fallback.** Its resolution changes in both `SharedConfig::from_env` and the server's `load_config`. The order is:
    1. `FRONTEND_BASE_URL`
    2. `FRONTEND_PUBLIC_URL`
    3. `server.toml`'s `frontend_base_url`
    4. the resolved app URL

    `http://localhost:3000` is no longer a fallback. Today the binary falls back to it even when `app_base_url` is set.
  - **Compose and the chart** already pass the app URL, so they don't change.
  - **chakramcp.com** sets `FRONTEND_PUBLIC_URL`, so it doesn't change either.
  - **Startup warning (`self_hosted` only).** When the frontend URL isn't the app URL, the server logs that sign-in links point elsewhere and not at its own pages. This flags 0.2.0 configs, where `init` wrote `frontend_base_url = "http://localhost:3000"` (§8.3).
- **The app origin.** This means the scheme, host and port of the resolved app URL (`APP_PUBLIC_URL`, or `server.toml`'s `app_base_url`). It's used for the `Origin` check, the cookie's `Secure` flag, `return_to` and `/qr`.
- **Page header:** the app's host name, e.g. `app.example.com`, so people can see which network they're signing in to.

### 6.2 `/oauth/authorize`: sign in and consent

**Request checks**, the same as the web UI's:
- `response_type=code`;
- a registered `client_id`;
- a `redirect_uri` that exactly matches a registered one;
- an S256 `code_challenge` of 43–128 characters;
- `scope=relay.full` (also the default);
- `state` passed through;
- the `agent_scope` and `agent_ids` hints.

**Errors:**
- An unknown client or an unregistered `redirect_uri` shows an error page and never redirects (RFC 6749 §4.1.2.1).
- Other invalid parameters redirect to the verified `redirect_uri` with `error=invalid_request` and `state`.

**Signed out:** a sign-in form (email and password). When sign-up is open, it also links to `/signup` and carries the request along.

**Signed in:** a consent page, with "Not you? Sign out". It shows:
- the client's name and URI, and the host it will redirect to;
- what the token allows: calling the relay's tools as this user, for 24 hours;
- the agent access choice:
  - all agents (the default, unless the client asked for another);
  - only agents the client creates;
  - selected agents, as a checklist of the user's own agents (agents in accounts the user is a member of).

**Approve and deny:**
- **Approve** issues a code through the function behind `POST /oauth/issue-code`, with the same checks. It then answers `303` to `redirect_uri?code=…&state=…`, leaving out `state` when it's empty.
- **Deny** answers `303` to `redirect_uri?error=access_denied&state=…`.

### 6.3 `/signup`, `/app/pair`, `/qr`, `/logout`

- **`/signup`** (only when sign-up is open):
  - Collects name, email and password, and calls `create_user`.
  - Then signs the user in and continues to `return_to`, or shows "Account created" when there's none.
  - **`return_to` checks:**
    - It must be a path.
    - It's resolved against the app URL and must keep the app origin.
    - Control characters and backslashes are refused, so tricks like `/%09/evil.com` fail.
- **`/app/pair`:**
  - Asks for the code, prefilled from `?session=`, and signs the user in if needed.
  - Shows the device request: client and hints.
  - Offers two choices, prefilled from the hints:
    - link one of the user's agents;
    - create one, with a slug, a name, an optional description, and visibility `private`, `org` or `network`.
  - Approve and deny call the same functions as `POST /oauth/device-approve` and `/oauth/device-deny`.
- **`/qr?data=<url>`:**
  - Renders an SVG QR code of the URL, with the URL as text below it.
  - It only accepts URLs on the app's own origin, so it can't be used to make QR codes for other sites.
- **`POST /logout`:** revokes the page session and clears its cookie.

### 6.4 Sessions and form security

- **Session cookie** `chakramcp_page_session`:
  - A JWT signed with a key derived from `JWT_SECRET` (HMAC-SHA256 of `JWT_SECRET` with a fixed label). It therefore isn't accepted as an API token.
  - Non-admin claims, a 1-hour lifetime.
  - `HttpOnly`, `SameSite=Lax`, `Path=/`, and `Secure` when the app URL is `https`.
  - Sign-out records its id in `revoked_tokens`, and the pages check that table.
- **CSRF:**
  - A random `chakramcp_csrf` cookie (`HttpOnly`, `SameSite=Strict`) is mirrored in a hidden form field.
  - Every POST compares the two in constant time.
  - **`Origin` header:** when present, it must equal the app origin; `Origin: null` is refused. When it's absent, as with curl or some older clients, the token decides.
- **Response headers:**
  - `Content-Security-Policy: default-src 'none'; style-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`. On the consent page, `form-action` also allows the verified redirect origin, because the approve step redirects there.
  - `X-Frame-Options: DENY` and `Cache-Control: no-store`.
  - `Referrer-Policy: same-origin`.
    - **Not `no-referrer`:** under that policy, browsers send `Origin: null` even on same-origin form posts, which would break the `Origin` check.
    - `same-origin` still sends no `Referer` to the client's redirect or its `client_uri`, so codes in URLs don't leak.
- **Rendering:** no JavaScript. Templates escape HTML automatically (askama), and the QR code is rendered by the `qrcode` crate as an SVG.

### 6.5 Failed sign-in limit (all modes)

- **Where:** the pages and `POST /v1/auth/login`. The web UI's password sign-in calls the latter, so chakramcp.com gets the limit too.
- **Rule:** after 10 failed attempts for one email within 15 minutes, further attempts for that email are refused until the window ends, even with the right password.
  - A successful sign-in clears the count.
  - Unknown emails count the same way, so the response can't be used to discover accounts.
- **Responses:**
  - **The limit:** `429` with error code `signin_rate_limited` and `Retry-After` in seconds.
  - **Sign-up while closed (§3):** `403` `signup_disabled`.
  - **Code changes:** the shared `ApiError` gains both variants and a way to set `Retry-After`. The existing 429 codes keep their meanings.
- **Storage:** Postgres, so it works without Redis and across replicas.
  - Migration `0037_signin_failures.sql` adds `signin_failures (email_hash BYTEA PRIMARY KEY, failures INT NOT NULL, window_started_at TIMESTAMPTZ NOT NULL)`, plus an index on `window_started_at`.
  - The key is the SHA-256 of the lowercased email, so addresses typed by strangers aren't stored.
  - The next attempt resets an expired row. Each failed attempt also deletes rows whose window ended more than a day ago.
- **Trade-off:** someone who knows an email can block password sign-in for that account, 15 minutes at a time. Existing sessions, OAuth tokens and API keys keep working, and operators can still use §4. The docs say so.

### 6.6 `chakramcp api-keys` (CLI)

- **Commands:**
  - `create --name <name> [--account <slug>] [--expires-in-days <n>] [--agent-scope all|own]`, which prints the key once. The endpoint takes an account id, so the CLI looks up the slug in the user's memberships from `/v1/me`;
  - `list`;
  - `revoke <id>`.
- They wrap the existing `/v1/api-keys` endpoints, using the CLI's current sign-in, so SDK users can get keys without a dashboard.
- They work against chakramcp.com too.

## 7. MCP sign-in discovery (all modes)

- **401 header:** the relay's 401 responses carry `WWW-Authenticate: Bearer resource_metadata="<relay URL>/.well-known/oauth-protected-resource"`, as the MCP authorization spec requires.
- **Path-form metadata:** the protected-resource metadata is also served at `/.well-known/oauth-protected-resource/mcp`, RFC 9728's path form for the `/mcp` resource.
- **CORS:** the relay's CORS layer exposes `WWW-Authenticate`, so browser-based MCP clients can read it.
- Stricter MCP clients need all three. chakramcp.com benefits too.

## 8. Configuration and docs

### 8.1 Compose (`infra/docker-compose.prod.yml`, `infra/.env.example`)

- **Settings passed to `relay`:** `HOSTING_MODE`, `SIGNUP_ENABLED`, `CREDITS_ENABLED`, `CREDITS_DEFAULT_MONTHLY_FREE_MC`, `CREDITS_COST_PER_INVOCATION_MC` and `LIMITS_DEFAULT_RATE_PER_MIN`, each defaulting to empty, which means the server's default.
- **`ADMIN_EMAIL`:** becomes `${ADMIN_EMAIL:-}`.
- **`.env.example`:**
  - The required group becomes the domains, `APP_PUBLIC_URL`, `RELAY_PUBLIC_URL`, `POSTGRES_PASSWORD` and `JWT_SECRET`.
  - `HOSTING_MODE`, `ADMIN_EMAIL`, `UPSERT_SHARED_SECRET` and `FRONTEND_PUBLIC_URL` move to a "chakramcp.com only" group. Compose still defaults `FRONTEND_PUBLIC_URL` to `APP_PUBLIC_URL`.
  - New groups for sign-up and credits.
- **Settings the server never reads are removed** from Compose and `.env.example`: `GOOGLE_*`, `GITHUB_*`, `RECAPTCHA_*` and `CAPTCHA_ENABLED`. Production's `.env` can keep them; nothing reads them.
- **Stale comments fixed:** the one about Caddy routing `/oauth/*`, and the one about `CREDITS_*`.

### 8.2 Helm chart

- **New values:** `config.signupEnabled` and `config.creditsEnabled`. Unset means the server's default. Credit amounts go through `extraEnv`, which the chart README documents.
- **`config.adminEmail` stays** so old values files still validate. The README says it has no effect on self-hosted servers.
- **`NOTES.txt`** prints the exact `kubectl exec deploy/<name> -- chakramcp-server users add …` command for the release. The deployment name depends on the release name.
- **The bundled `helm test`** also checks that `/oauth/authorize` is served: any status but 404.

### 8.3 Single binary

- **`chakramcp-server init`:**
  - It stops writing `frontend_base_url`, so the server falls back to the app URL (§6.1). It writes commented lines for the new keys.
  - `--admin-email` still writes `admin_email`, which only `managed` mode uses.
  - It ends by printing the next steps: `migrate`, `start`, and `users add <email> --admin`.
- **`server.toml`** accepts `hosting_mode`, `signup_enabled` and `credits_enabled` (§3), plus the credit, limit and Redis settings (§5).
- **Upgrade note:**
  - Configs written by 0.2.0's `init` contain `frontend_base_url = "http://localhost:3000"`. Delete that line. The server warns until you do (§6.1).
  - The new order also puts that line ahead of an `APP_PUBLIC_URL` set in the environment, so delete it in that case too.
- **Homebrew:** both `Formula/chakramcp-server.rb` and `packaging/server/homebrew/chakramcp-server.rb.template` get updated caveats:
  - the real config path;
  - `users add … --admin` between `start` and `chakramcp login`;
  - no "edit `admin_email`" step.
- The docs and the Homebrew formula's caveats give the real default config path, which the code takes from `directories::ProjectDirs`:
  - Linux: `~/.config/chakramcp/server.toml`
  - macOS: `~/Library/Application Support/com.chakramcp.chakramcp/server.toml`

  The code isn't changed, so existing configs keep working.

### 8.4 Docs

- **`docs/self-hosting/compose.md`, first start:**
  1. Configure, then start.
  2. `users add … --admin`.
  3. `chakramcp networks add private …`, then `chakramcp networks use private`. A fresh CLI points at the public network, and `networks add` doesn't switch.
  4. `chakramcp login`: the browser opens the server's sign-in page.
  5. Connect MCP clients.
  6. Add teammates with `users add`, or set `SIGNUP_ENABLED=true`.
  7. Get API keys for SDKs with `chakramcp api-keys create`.
- **New sections:**
  - "Credits": off by default, how to turn them on, the commands.
  - "Accounts": admins, passwords, the sign-in limit.
- **compose.md's local-trial paragraph:** it switches to plain `http://app.localhost` and `http://relay.localhost`, the setup CI uses. The CLI won't trust Caddy's local certificate authority, so the old `https://*.localhost` setup can't sign in.
- **The root `README.md` "Self-hosting" section:** the config path, and the `users add` step.
- **The other guides:**
  - `kubernetes.md`: the same steps with `kubectl exec`.
  - `docs/INSTALL.md` (binary): the same steps, plus the config-path fix.
  - `docs/self-hosting/README.md`: sign-in pages are included; a dashboard isn't.
- **The website's `/docs/self-host` page:** link to the guides, and fix what will be wrong:
  - `NEXT_PUBLIC_RELAY_API_URL` should be `NEXT_PUBLIC_RELAY_URL`. The same mistake is in `frontend/src/app/(app)/app/page.tsx` and `frontend/src/app/(site)/docs/concepts/page.tsx`;
  - `ADMIN_EMAIL` is no longer how a self-hosted admin is made;
  - the `server.toml` path.
- **`docs/CI-CD.md`:** the `HOSTING_MODE=managed` requirement, CD's guard, and the new workflow.
- **The credit ledger spec:** mark the first-self-hoster checklist done, and note that `HOSTING_MODE` arrived here.

### 8.5 Local development and examples (PR-2)

Two changes would break local work against a local server: closed sign-up, and the new frontend URL fallback (§6.1). Local dev sets no frontend URL today and relies on the old `http://localhost:3000` default. PR-2 updates each caller:
- **Root `.env.example` and the Taskfile's `dev:backend` task:** set `HOSTING_MODE=managed` and `FRONTEND_BASE_URL=http://localhost:3000`.
  - Local development mirrors chakramcp.com, with the web UI on `:3000` and sign-up open.
  - The `ADMIN_EMAIL` comment there now says it applies only in `managed` mode.
- **`frontend/e2e/README.md`:**
  - the local server it expects runs with both settings;
  - `chakramcp-server` doesn't load `.env` files, so they must be exported or put in `server.toml`.
- **`examples/scheduler-demo` and `examples/hermes-openclaw-demo`:** their READMEs tell Homebrew-server users to set `signup_enabled = true`. Their `setup.py` scripts print that hint when sign-up returns `signup_disabled`.

## 9. CI: `self-host-e2e.yml` (new)

**Triggers:**
- It runs on every pull request and every push to main.
- `dorny/paths-filter` skips the steps unless the change touches `backend/**`, `infra/docker-compose.prod.yml`, `infra/Caddyfile`, `infra/Dockerfile.release`, `docs/self-hosting/**` or the workflow. This follows chart-ci and observability-ci, so the check can become required without leaving other PRs waiting on it.

**Build:**
- Runs on `ubuntu-22.04`, like backend-ci and CD. Its glibc is older than the image's bookworm, so the binary runs there.
- `cargo build --release --locked` of the server and the CLI, with `SQLX_OFFLINE=true` and the Rust cache.
- The server goes into an `infra/Dockerfile.release` image, the one self-hosters pull, tagged `chakramcp-server:e2e`.

**Setup:**
- A generated `.env`:
  - `APP_DOMAIN=http://app.localhost` and `RELAY_DOMAIN=http://relay.localhost`, with matching `http://` URLs;
  - `LOG_DRIVER=json-file`;
  - `CHAKRAMCP_IMAGE=chakramcp-server:e2e`;
  - random secrets.
- `/etc/hosts` maps both names to 127.0.0.1.
- Plain HTTP is deliberate. The CLI trusts only public certificate authorities, so Caddy's local one won't do. Everything else matches compose.md.

**Steps,** in compose.md's order:
1. `docker compose -f docker-compose.prod.yml up -d --wait`, then check health.
2. `users add admin@example.test --admin --password-stdin`.
3. `chakramcp networks add`, `chakramcp networks use`, then `chakramcp login --method browser`.
   - `BROWSER` points at a script that drives the pages in the background: sign in, then approve.
   - It's written in Python with the standard library, and sends `Origin` like a browser.
   - It logs to a file, because the `webbrowser` crate discards the browser's output.
4. `chakramcp whoami` shows the admin.
5. `chakramcp api-keys create`, and the key authenticates `GET /v1/me`.
6. `chakramcp pair --json`, then approve on `/app/pair` by creating an agent. The CLI receives its token.
7. As an MCP client:
   1. an unauthenticated `POST /mcp` returns 401 with `WWW-Authenticate`;
   2. fetch both metadata documents;
   3. register a client;
   4. authorize and consent through the pages;
   5. exchange the code with PKCE;
   6. call `initialize` and check that `serverInfo.version` is set;
   7. call `tools/list`.
8. With the defaults, `POST /v1/auth/signup` returns 403 `signup_disabled`, and `credits show` reports `off`.
9. On failure, print the logs.

Once it's stable, it's added to the required checks (ask the user then).

## 10. Testing summary

| Layer | How |
|---|---|
| Unit and integration (`backend-ci`) | <ul><li>**Config:** parsing of the mode and the overrides; frontend URL resolution, including the old `localhost:3000` case and local dev's `FRONTEND_BASE_URL`.</li><li>**Credits off:** gate skip, worker discard, views.</li><li>**Sign-up and admins:** sign-up closed; upsert refusing new users while sign-up is closed; `ADMIN_EMAIL` ignored on self-hosted sign-up and upsert.</li><li>**Operator commands** against the test database.</li><li>**Page flows** through axum requests: sign-in; CSRF rejections (a missing token, a foreign `Origin`, `Origin: null`); consent approve with the code redeemed at `/oauth/token` with PKCE; deny; redirect validation; `return_to` tricks; pairing approve; the `/qr` origin check; logout revocation.</li><li>**The sign-in limit**, including `Retry-After`.</li><li>**MCP discovery:** the 401 header and the CORS exposure.</li></ul> |
| Compose end to end | `self-host-e2e.yml` (§9) |
| Chart | `chart-ci.yml`'s `helm test` checks the page (§8.2) |
| Production | After each deploy: <ul><li>the startup log reads `hosting_mode=managed credits=on signup=open`;</li><li>`app.chakramcp.com/oauth/authorize` still answers 404;</li><li>an invalid sign-up body still gets a validation error, not `signup_disabled`;</li><li>after PR-3, an unauthenticated `POST relay.chakramcp.com/mcp` carries `WWW-Authenticate`.</li></ul> |

## 11. Rollout (PRs)

1. **PR-1, operator commands:** §4 with its refactors, and this spec and the plan. Behaviour doesn't change.
2. **PR-2, `HOSTING_MODE`:** §3, §5, the frontend URL fallback from §6.1, §8.1–8.3, §8.5 and CD's guard.
   - **The guard** runs before cd.yml's "Sync infra config to VM" step. That way a missing `HOSTING_MODE=managed` stops the deploy before a Compose file defaulting to `self_hosted` reaches the VM.
   - **Before merge:**
     - add `HOSTING_MODE=managed` to production's `.env`;
     - render the Compose config on the VM and compare it with today's.
   - **After merge:** check the startup log line (§10).
3. **PR-3, built-in pages:** §6 (including the sign-in limit and `chakramcp api-keys`) and §7.
   - **After merge:** the §10 production checks.
4. **PR-4, CI and docs:** §9, the §8.2 `helm test` check, and §8.4.
5. **Release 0.3.0** (confirm the version with the user).
   - The release notes call out the self-host changes: sign-up closed by default, admins via `users`, credits off.

## 12. Risks

| Risk | Mitigation |
|---|---|
| chakramcp.com switches to self-hosted defaults | `HOSTING_MODE=managed` goes into `.env` before PR-2; CD refuses to deploy without it; the startup log is checked after deploy |
| The pages add an attack surface | No JavaScript; strict CSP; CSRF tokens; no framing; exact redirect matching; a session key that can't be used against the API; same-origin `return_to` and `/qr`; tests for each |
| The sign-in limit is used to lock someone out | Failures only, 15 minutes; other credentials keep working; operators can still act through §4 |
| 0.2.0 self-hosters relied on `ADMIN_EMAIL` | Existing admins keep the flag; `users set-admin` covers new ones; the release notes say so |
| Turning credits on after running without them | The worker never charged while credits were off. Accounts without a wallet start fresh. Older wallets keep their balance and catch up on missed grants (up to 12 months), as §5 documents. |
| Empty values from Compose stop startup | Empty counts as unset for every new and credit setting |
| Local development, the examples and the web UI's e2e suite sign up against a server whose sign-up is now closed, or lose the `localhost:3000` frontend default | §8.5 updates each one in the same PR, setting `HOSTING_MODE=managed` and `FRONTEND_BASE_URL` for local dev |
| 0.2.0 binary configs keep pointing sign-in at `localhost:3000` | New frontend URL fallback; a startup warning; the upgrade note |
| End-to-end flakiness | `--wait`, health retries, logs on failure; required only once it's stable |
