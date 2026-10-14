# CI/CD

## At a glance

```
   PR opened ──▶ pre-merge checks ──▶ branch protection gate ──▶ merge ──▶ CD
                  • Frontend CI                                        • detect changes
                  • CLI CI                                             • deploy-frontend (if frontend/**)
                  • sqlx prepare check                                 • deploy-backend (if backend/** or infra/**)
                  • CodeQL                                             • run migrations (if backend/migrations/**)
                  • Security scan (gitleaks blocks; rest advisory)     • probe /healthz on relay
                  • Lefthook pre-push: clippy + frontend lint all
```

## Pre-merge: already wired

Branch protection on `main` requires these checks green and the branch
up to date before a merge button enables. Inspect or change them at
<https://github.com/Delta-S-Labs/chakra_mcp/settings/branches>, or with
`gh api repos/Delta-S-Labs/chakra_mcp/branches/main/protection/required_status_checks`.

| Required check | Workflow | What |
|---|---|---|
| `Lint, typecheck, build` | `frontend-ci.yml` | The Next.js app |
| `Dependency audit` | `frontend-ci.yml` | `pnpm audit --audit-level moderate`: a fresh advisory can turn it red with no code change |
| `Build (ubuntu-22.04)` | `cli-ci.yml` | The CLI: build, clippy, fmt |
| `Verify .sqlx cache is up to date` | `sqlx-prepare-check.yml` | `cargo sqlx prepare --workspace --check -- --tests` |
| `Analyze (javascript-typescript)` | `codeql.yml` | SAST |
| `Lint + typecheck + test (3.12)`, `Lint + test + build`, `Lint + test`, `Vet + test (go 1.22)` | `sdk-py-ci.yml`, `sdk-ts-ci.yml`, `sdk-rust-ci.yml`, `sdk-go-ci.yml` | The four SDKs |
| `Observability config + smoke test` | `observability-ci.yml` | [Observability](#observability) |
| `Chart lint, render + kind install` | `chart-ci.yml` | [Helm chart](#helm-chart) |

A required check must report on every PR, so these workflows run on all
of them and skip their steps when nothing relevant changed.

`self-host-e2e.yml` (`Self-host end to end`) isn't required yet. It runs
on every PR and follows `docs/self-hosting/compose.md` with this commit's
server and CLI (`infra/e2e/run.sh`):
- start the stack;
- create the admin with `chakramcp-server users add`;
- sign in with `chakramcp login` through the server's own pages, driven by
  a scripted browser;
- create an API key and pair a device;
- connect as an MCP client from the relay's URL alone;
- check that sign-up is closed and credits are off.

It runs on plain-HTTP `*.localhost` names. Locally:
`CHAKRAMCP_IMAGE=<image> CHAKRAMCP_CLI=<chakramcp binary> infra/e2e/run.sh`
(on macOS the sign-in step pairs instead, because the CLI opens the
default browser there).

Plus `.github/workflows/security-scan.yml` runs on every PR and
**blocks** on leaked secrets via gitleaks. Other scans
(`cargo audit`, `pnpm audit`, `pip-audit`, ZAP) are advisory:
they post warnings rather than failing the workflow.

The lefthook `pre-push` hook also runs full-workspace clippy +
ESLint locally so red CI on a feature branch is rare. Activate
once per clone:

    task install:hooks

## Post-merge: CD pipeline

`.github/workflows/cd.yml` triggers on `push: main` (and manual
dispatch). It runs four jobs:

1. **detect**: `dorny/paths-filter` sets booleans for `frontend`,
   `backend`, and `migrations`. The downstream jobs gate on these.

2. **surface-frontend-changes**: if `frontend/**` changed, just
   posts a workflow annotation. **Netlify's GitHub integration
   handles the actual deploy** (auto-build on push to main + a
   deploy preview on every PR). No token required from us. If you
   ever need to force a redeploy without a code change, do it from
   the Netlify UI or via `npx netlify-cli deploy --prod --build`
   from your laptop's `netlify login` session.

3. **deploy-backend**: if `backend/**` or a top-level `infra/*` file
   (compose, Caddyfile, Dockerfiles) changed. `infra/observability/**`
   deliberately doesn't trigger it:
   - `cargo build --release --bin chakramcp-server` natively on
     the ubuntu-22.04 runner (no cross-compile, runner IS x86_64).
   - `cp target/release/chakramcp-server infra/chakramcp-server`.
   - `docker build -f infra/Dockerfile.thin` → `docker push` to
     `877326604850.dkr.ecr.us-east-1.amazonaws.com/chakramcp-server`
     with tags `${sha:0:7}` + `latest`.
   - **Check that production runs as managed.** The server defaults to
     `HOSTING_MODE=self_hosted`: credits off, public sign-up closed,
     `ADMIN_EMAIL` ignored. chakramcp.com needs `HOSTING_MODE=managed`
     in `/opt/chakramcp/.env`, and the deploy stops before touching the
     VM if that exact line is missing. To fix it, append the line (back
     the file up first; never `source` it) and re-run the workflow.
   - Copy `infra/docker-compose.prod.yml` (as `docker-compose.yml`)
     and the `Caddyfile` to `/opt/chakramcp`. `docker compose up -d
     --no-deps caddy` then recreates Caddy only when its definition
     changed (e.g. new environment for the Caddyfile's hostnames), and
     `caddy reload` applies a changed Caddyfile.
   - **If migrations changed** (`backend/migrations/**`):
     `docker compose --profile migrate run --rm migrate` over SSH
     to `ubuntu@54.84.88.246` (Lightsail prod). Runs BEFORE the
     relay restart so new code never sees an old schema.
   - `docker compose up -d --force-recreate relay`.
   - Probe `https://relay.chakramcp.com/healthz`, `/readyz`,
     `/v1/discovery/agents`. Fail the workflow if any return non-200.

4. **deploy-observability**: on every push (after deploy-backend, or
   when it was skipped; never after a failed one). It rsyncs
   `infra/observability/` to `/opt/chakramcp/observability/`, hashing
   each config directory before and after, then runs
   `observability/scripts/deploy.sh` on the VM. That script creates
   the Postgres monitoring role if it's missing, brings the four
   services up, and applies only what changed, reloading instead of
   restarting where it can; a changed alert channel file recreates
   Grafana, since it's a single-file mount. It fails the job if
   Prometheus or Alloy rejected the new config. When nothing changed it
   touches nothing.
   See [Observability](#observability).

### Required secrets

Set once via `gh secret set <NAME> --repo Delta-S-Labs/chakra_mcp`:

| Secret | What | How to get it |
|---|---|---|
| `NETLIFY_SITE_ID` | `ef540682-67b3-46e6-a425-afbe85437f88` | Stored for future workflow needs (cache clear, etc.); no current job uses it. |
| `AWS_ACCESS_KEY_ID` | IAM user creds for ECR push | Create user `cd-publisher` with `AmazonEC2ContainerRegistryPowerUser` policy |
| `AWS_SECRET_ACCESS_KEY` | Pair of above | Same user |
| `LIGHTSAIL_SSH_KEY` | Private key for `ubuntu@54.84.88.246` | Contents of `~/.ssh/lightsail-chakramcp-prod.pem` |
| `NPM_TOKEN` | Automation token for `npm publish` | Already set: npmjs.com → Access tokens → Automation |
| `TELEGRAM_BOT_TOKEN` | `@chakramcp_bot`'s token, for `uptime.yml` | Same value as the VM's `.env` (BotFather) |
| `TELEGRAM_CHAT_ID` | The chat alerts go to, for `uptime.yml` | Same value as the VM's `.env` |

No `NETLIFY_AUTH_TOKEN` needed: Netlify deploys via its GitHub
integration, not via our workflow.

OIDC trust to AWS is the production upgrade path. It replaces the
long-lived access key with a per-run token. Configure the OIDC
provider in IAM, create a role trusted by
`token.actions.githubusercontent.com` scoped to
`repo:Delta-S-Labs/chakra_mcp:ref:refs/heads/main`, then change the
`Configure AWS credentials` step to use `role-to-assume`.

### Manual deploy

```
gh workflow run cd.yml -f which=both       # both surfaces
gh workflow run cd.yml -f which=frontend   # only frontend
gh workflow run cd.yml -f which=backend    # only backend
gh workflow run cd.yml -f which=observability  # only the observability stack
```

Useful when:

- You want to redeploy without a code change (e.g. flip env vars
  and want them in the bundle).
- A deploy failed mid-step and you fixed the env without
  triggering a re-merge.

## Public image

`ghcr.io/delta-s-labs/chakramcp-server` (linux/amd64 + linux/arm64), for
self-hosters. Production keeps its private ECR image.

- **`:edge` / `:sha-<7>`**: [`image.yml`](../.github/workflows/image.yml),
  on every push to main that changes `backend/**`, the Dockerfile or the
  workflow. PRs that change the Dockerfile or workflow build both
  architectures without pushing.
- **`:X.Y.Z` (+ `:X.Y`, `:latest` for final releases)**: the `image` and
  `image-merge` jobs of `cli-release.yml`, from the release's own linux
  server binaries.

Each architecture builds natively (`ubuntu-22.04` / `ubuntu-22.04-arm`,
whose glibc 2.35 is older than the `debian:bookworm-slim` base's 2.36),
pushes a single-platform image by digest, and a merge job joins them into
the multi-arch tags. There's no emulation anywhere. Images carry build
provenance and an SBOM. Builds set `CHAKRAMCP_VERSION` (`edge` or the
release) and `GIT_SHA`, which `chakramcp-server --version` and the
`chakramcp_build_info` metric report.

**One-time:** after the first push, make the package public
(GitHub → the org's Packages → `chakramcp-server` → Package settings →
Change visibility) if it didn't inherit the repo's visibility.

## Helm chart

`charts/chakramcp` ([guide](self-hosting/kubernetes.md)).

- **`chart-ci.yml`** runs on every PR and skips itself unless `charts/**`,
  the dashboards and rules in `infra/observability/grafana/`, or the
  workflow changed:
  - check that `charts/chakramcp/files/` matches its source: run
    `infra/observability/scripts/sync-chart-assets.sh` after changing a
    dashboard or rule;
  - lint and render with each values file in `ci/` (installable) and
    `ci-template/` (render-only, with the ServiceMonitor API);
  - check that invalid values are refused;
  - validate the manifests with kubeconform against Kubernetes 1.37 and
    1.33;
  - install every `ci/` file on kind with `ct` and run `helm test`. The
    namespace enforces the `restricted` Pod Security profile, and the
    `:edge` image is loaded into kind with the workflow's token.
- **Releases.** The `chart` job of `cli-release.yml` packages the chart
  with `--version` and `--app-version` set to the release, and pushes it
  to `oci://ghcr.io/delta-s-labs/charts`. It runs after the image tags
  exist. No `:edge` chart is published: unreleased changes install from
  a checkout.
- **Subcharts** (the bundled observability stack) are pinned in
  `Chart.yaml` and `Chart.lock`. The vendored `charts/` directory isn't
  committed; `helm dependency build` fetches it after `helm repo add` for
  prometheus-community and grafana (Loki and Grafana are OCI). Dependabot
  watches them, but its support for `oci://` dependencies is uneven. If
  Loki or Grafana never get PRs, bump them by hand: edit the versions,
  `helm dependency update charts/chakramcp`, and commit `Chart.lock`.
- **One-time:** the chart package needs the same public-visibility toggle
  as the image.

## Observability

Metrics, logs, dashboards and Telegram alerts for production. Designs:
[phase 1](superpowers/specs/2026-09-29-observability-phase1-design.md)
and, for the self-hosting settings,
[phase 2](superpowers/specs/2026-09-29-self-hosting-phase2-design.md).
Self-hosters run the same files; their guide is
[`docs/self-hosting/`](self-hosting/README.md). Everything specific to
production is in the VM's `.env`.

- **Grafana:** <https://grafana.chakramcp.com>. GitHub sign-in is the
  only way in (the login form is off); the allowlist is
  `GRAFANA_ROLE_ATTRIBUTE_PATH` in the VM's `.env`.
- **The stack:** `infra/observability/compose.yml`, an overlay on the prod
  compose file.
  - **Alloy** scrapes every metrics source and reads container logs from
    the host journal.
  - **Prometheus** stores metrics for 15 days, capped at 2 GB.
  - **Loki** stores logs for 14 days.
  - **Grafana** serves the dashboards and alerts.
- **Dashboards and alert rules** are files in `infra/observability/grafana/`.
  Change the files, not the UI; CD deploys them.
- **Alerts** go to Telegram via `@chakramcp_bot`: `ALERT_CHANNEL=telegram`
  selects `infra/observability/grafana/channels/telegram.yml`.
  `.github/workflows/uptime.yml` checks the public endpoints every
  10 minutes from GitHub, so a dead VM still raises one.

### On the VM

The overlay needs both files on every compose command. Nothing in `.env`
adds it (deliberately), so:

```
cd /opt/chakramcp
alias obs='docker compose -f docker-compose.yml -f observability/compose.yml'
obs ps alloy prometheus loki grafana
obs logs --tail 50 grafana
```

A plain `docker compose` against the base file lists the observability
containers as orphans; that's harmless. Don't use `--remove-orphans`.

**`.env` keys** (mode 600; read single keys with `grep`/`cut`, and
never `source` the file, because the allowlist value has quotes and
backticks):

| Key | What |
|---|---|
| `APP_DOMAIN`, `RELAY_DOMAIN`, `GRAFANA_DOMAIN` | The Caddy sites (and Grafana's root URL) |
| `ALERT_CHANNEL`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID` | Where alerts go: `telegram` and its bot |
| `PROBE_TARGETS` | The HTTPS probes, a JSON list: relay, app, frontend, grafana |
| `GRAFANA_GITHUB_ENABLED`, `GRAFANA_GITHUB_AUTO_LOGIN`, `GRAFANA_DISABLE_LOGIN_FORM` | `true` each: GitHub is the only sign-in |
| `GRAFANA_GITHUB_CLIENT_ID`, `GRAFANA_GITHUB_CLIENT_SECRET` | The `chakramcp-grafana` GitHub OAuth app (callback `https://grafana.chakramcp.com/login/github`) |
| `GRAFANA_ROLE_ATTRIBUTE_PATH` | Who may sign in: a JMESPath over GitHub's `/user` response that yields `'GrafanaAdmin'`, or `''` to refuse. Match on the numeric `id`, written as a number literal (`` id == `123` ``). |
| `GRAFANA_ADMIN_PASSWORD` | Break-glass admin. Basic auth is refused at Caddy, so it only works from inside the VM. |
| `PG_MONITOR_PASSWORD` | The `chakramcp_monitor` role (`pg_monitor`: read-only statistics) that Alloy logs in as |

**Break-glass access** (GitHub sign-in broken): tunnel to Grafana's
container and sign in as `admin`.

```
ip=$(ssh ubuntu@54.84.88.246 "docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}' chakramcp-grafana-1" | cut -d' ' -f1)
ssh -L 3000:$ip:3000 ubuntu@54.84.88.246
```

Then browse to <http://localhost:3000/login?disableAutoLogin=true>. The
login form is off, so use the API with basic auth there, or set
`GF_AUTH_DISABLE_LOGIN_FORM=false` temporarily. The same tunnel works
for Prometheus (`:9090`) and Loki (`:3100`).

**Rotating the monitoring password:**
1. Set `PG_MONITOR_PASSWORD` in `.env`.
2. Run `obs exec -T pg psql -U chakramcp -d chakramcp`, then
   `ALTER ROLE chakramcp_monitor PASSWORD '…';`
3. Run `obs up -d --force-recreate alloy`.

CD only sets the password when it creates the role.

### Locally

`infra/observability/compose.dev.yml` runs the stack on a laptop; its
header has the commands. It needs a Docker runtime with journald: Linux
or colima work, Docker Desktop doesn't. The observability CI
(`observability-ci.yml`) validates every config (all five alert
channels), checks that `deploy.sh` refuses a host without journald, and
runs a smoke test of the real stack with the generic `ci.env` settings,
deployed by the same `deploy.sh` next to the relay from `:edge`.

### If the uptime workflow stops

GitHub disables scheduled workflows in public repos after 60 days
without repository activity, and emails the owner. Re-enable it in the
Actions tab. Test the alert path any time:

```
gh workflow run uptime.yml -f test_message=true
```

## Dependabot

`.github/dependabot.yml` watches:

- Frontend npm (grouped: react, nextjs, types)
- TS SDK npm
- Rust workspace (grouped: tokio-ecosystem, sqlx, axum)
- Rust SDK
- Python SDK pip
- Go SDK go.mod
- GitHub Actions

`.github/workflows/dependabot-auto-merge.yml` runs on each
dependabot PR. After CI green, it **auto-merges**:

- Any patch update (semver-patch)
- Minor dev-deps (e.g. `@types/*`, `eslint`, test runners)
- Indirect deps (transitive)

It **holds for human review**:

- Major-version bumps (you can break)
- Minor runtime production deps (`next`, `react`, `sqlx`, etc.)

Held PRs get the `needs-human-review` label so they're easy to filter.

## Pending PRs at the time of writing

PRs that were red before the CI-fix batch landed need a rebase:

- #11 `@types/node` 20→25: major-version bump for dev-deps, will hold
- #12 `next` group bump: patch/minor; auto-merge after rebase
- #13 `react` group bump: likely patch; auto-merge after rebase

Trigger a re-test by commenting `@dependabot recreate` on each PR.

## Local dev mirrors CI

Every check CI runs has a local equivalent:

| CI step | Local |
|---|---|
| Frontend CI lint | `cd frontend && npx eslint "src/**/*.{ts,tsx}"` |
| Frontend typecheck | `cd frontend && npx tsc --noEmit -p .` |
| Frontend build | `cd frontend && pnpm build` |
| CLI clippy | `cd backend && cargo clippy -p chakramcp-cli -- -D warnings` |
| CLI fmt | `cd backend && cargo fmt -p chakramcp-cli --check` |
| sqlx prepare check | `cd backend && cargo sqlx prepare --workspace --check -- --tests` |
| Workspace clippy | `cd backend && cargo clippy --workspace -- -D warnings` |

Lefthook runs the first six automatically on `pre-commit`; the
last (full workspace clippy) runs on `pre-push`.
