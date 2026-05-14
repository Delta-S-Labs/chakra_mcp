# ChakraMCP — Playwright local smoke test

End-to-end coverage for the full **auth → pair → API key → invoke →
dashboard** loop, driven against a manually-started local stack.

The single spec (`auth-and-dashboard.spec.ts`) runs in one ordered
sequence, organised into `test.describe` blocks per phase. It is
intentionally **local-only** — no CI wiring lives here yet.

## What the test covers

| Phase | What happens | UI evidence |
|-------|--------------|-------------|
| 1 | Email-only signup against `/v1/auth/signup`, then UI password sign-in. Dashboard renders with the user's first name + stat-card grid. | `phase1-dashboard.png` |
| 2 | Device-flow pairing. `POST /oauth/device_authorization`, navigate the authed browser to `verification_uri_complete`, accept the prefilled slug, approve. Token poller validates the access token. | `phase2-consent.png`, `phase2-approved.png`, `phase2-device-auth.json` |
| 3 | API key creation through `/app/api-keys` UI. Extract `ck_…` plaintext from the one-time-reveal panel and confirm it auths `/v1/me`. | `phase3-key-reveal.png` |
| 4 | Populate real invocation data: create a peer agent + echo capability, propose + accept a friendship, issue a grant, invoke the capability 4× **with the API key as caller** (so each row stamps `api_key_id`), then drain the peer's inbox. | (no UI screenshot — backend-only) |
| 5 | Navigate to `/app/api-keys/<id>`, assert recharts visualisations render: `Total > 0`, capability legend lists `echo`, by-agent table has the device-flow agent slug. | `phase5-dashboard.png` |
| 6 | `afterAll` cleanup: revoke API key, revoke pairing, tombstone both agents, log the orphaned user row. Idempotent — every step is wrapped in `try/catch`. | (cleanup log on stdout) |

## Prerequisites

### Toolchain
- Node 20+ and `pnpm`
- Rust toolchain (for the backend)
- Docker (for Postgres)
- Bundled Playwright Chromium (one-time: `pnpm e2e:install`)

### Environment

The frontend's password sign-in form is captcha-gated by default. The
test cannot solve a reCAPTCHA v2, so the test setup **must** run the
frontend dev server with captcha disabled:

```bash
# frontend/.env.local — add or set:
CAPTCHA_ENABLED=false
```

Backend env (`.env` or `.env.local` at repo root or `backend/.env`):

```
DATABASE_URL=postgres://chakramcp:chakramcp@localhost:5432/chakramcp
JWT_SECRET=<openssl rand -hex 32>
# Optional but recommended for tests:
SURVEY_ENABLED=false
```

## Startup order

Three terminal windows. Order matters — the test fails fast in
`waitForStack()` if any of the three is missing.

```bash
# 1. Postgres (one-time per machine session)
task db:up
#    or: cd backend && docker compose up -d postgres

# 2. Backend (app on :8080, relay on :8090; migrations run on startup)
cd backend
cargo run -p chakramcp-server -- start

# 3. Frontend dev server on :3000 — captcha MUST be disabled
cd frontend
CAPTCHA_ENABLED=false pnpm dev
```

## Running the test

```bash
cd frontend

# First time: install the bundled Chromium.
pnpm e2e:install

# Run the full spec, headless.
pnpm e2e

# Or interactively (Playwright UI mode).
pnpm e2e:ui
```

Override URLs if you bind the stack to non-default ports:

```bash
E2E_APP_URL=http://localhost:9080 \
E2E_RELAY_URL=http://localhost:9090 \
E2E_FRONTEND_URL=http://localhost:3001 \
  pnpm e2e
```

## Where artefacts land

Every `pnpm e2e` run creates a fresh dir under
`frontend/e2e/screenshots/<ISO-timestamp>/` containing:

- `phase1-dashboard.png` — signed-in dashboard
- `phase2-consent.png` — agent-pair consent screen
- `phase2-approved.png` — pair success state
- `phase2-device-auth.json` — raw `/oauth/device_authorization` response
- `phase3-key-reveal.png` — API-key one-time reveal panel
- `phase5-dashboard.png` — `/app/api-keys/<id>` with recharts rendered
- `auth-storage-state.json` — NextAuth cookie persisted from Phase 1 (re-used by Phase 2+)
- `test-results/` — Playwright's per-test trace.zip, video.webm, error
  context for failed tests
- `html-report/` — open with `pnpm exec playwright show-report frontend/e2e/screenshots/<run>/html-report`

## Cleanup notes

`test.afterAll` in the spec tears down everything it created, in reverse
order, with each step wrapped in `try/catch` so partial failures don't
strand the rest. After a successful run you should see no `e2e-*`
agents in the dashboard.

**Known orphan:** the test user row is **not** cleaned up.
`chakramcp-app` does not currently expose a `DELETE /v1/users/me`
endpoint, so each run leaves a `smoke-test-<timestamp>@chakramcp.local`
user behind. Manual cleanup if it bothers you:

```bash
task db:psql
-- Inside psql:
DELETE FROM users WHERE email LIKE 'smoke-test-%@chakramcp.local';
-- Account rows + memberships cascade.
```

The `Date.now()`-suffixed email guarantees uniqueness across runs, so
repeated runs don't collide on the email unique constraint — but they
do accumulate orphans. Drop the lot in one query when needed.

## Config decisions

See `playwright.config.ts` for the source of truth. Headlines:

- **`workers: 1`, `fullyParallel: false`** — the spec is one ordered
  sequence (signup → pair → key → invoke → dashboard → cleanup).
  Parallel runs would race the shared `state` bag.
- **`retries: 0`** — a retry on a half-cleaned-up failure would just
  create more orphans. If a phase fails, fix it and re-run.
- **`screenshot: 'on'`** — every test gets an auto-PNG at the end, in
  addition to the explicit milestone PNGs the spec writes. Cheap.
- **`video: 'retain-on-failure'`**, **`trace: 'retain-on-failure'`** —
  keeps disk usage low on the happy path; gives full visibility when
  something breaks.
- **`timeout: 180_000`** — generous, because the very first cargo build
  is slow and Phase 4 has its own latencies (4 invokes + inbox drain).
- **`test.describe.configure({ mode: "serial" })`** at the top of the
  spec — module-scope `state` is shared across `test()` invocations
  within one file *only* in serial mode. (Without it, Playwright is
  free to use multiple workers per file, which resets module state.)

## Troubleshooting

| Symptom | Likely cause |
|---------|--------------|
| `waitForStack` complains about backend at `:8080` | `cargo run -p chakramcp-server -- start` not running |
| `waitForStack` complains about frontend at `:3000` | `pnpm dev` not running |
| Phase 1: sign-in button stays disabled | `CAPTCHA_ENABLED` is true in `frontend/.env.local` — disable it |
| Phase 1: signup returns 409 | Stale orphan from previous run with the same `Date.now()` — only possible at sub-millisecond collisions; almost certainly a clock issue |
| Phase 2: `pollDeviceToken` budget exhausted | The approve click fired but didn't land. Open the trace to see why |
| Phase 5: `total = 0` | The invoke loop in Phase 4 failed silently. Check `pullInbox` returned the rows |

Open a trace for any failure with:
```bash
pnpm exec playwright show-trace frontend/e2e/screenshots/<run>/test-results/<test-id>/trace.zip
```
