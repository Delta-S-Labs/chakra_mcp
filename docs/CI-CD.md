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

## Pre-merge — already wired

Branch protection on `main` requires all four status checks green
and the branch up-to-date before a merge button enables. Set up via
`gh api PUT /repos/Delta-S-Labs/chakra_mcp/branches/main/protection`.
Inspect / change at <https://github.com/Delta-S-Labs/chakra_mcp/settings/branches>.

Required checks:

- `Frontend CI` — lint + typecheck + build for the Next.js app
- `CLI CI` — `cargo build` + `cargo clippy --workspace -- -D warnings` + `cargo fmt --check` on `backend/cli`
- `Verify .sqlx cache is up to date` — runs `cargo sqlx prepare --workspace --check -- --tests` so committed query JSON matches source
- `CodeQL` — SAST baseline

Plus `.github/workflows/security-scan.yml` runs on every PR and
**blocks** on leaked secrets via gitleaks. Other scans
(`cargo audit`, `pnpm audit`, `pip-audit`, ZAP) are advisory —
they post warnings rather than failing the workflow.

The lefthook `pre-push` hook also runs full-workspace clippy +
ESLint locally so red CI on a feature branch is rare. Activate
once per clone:

    task install:hooks

## Post-merge — CD pipeline

`.github/workflows/cd.yml` triggers on `push: main` (and manual
dispatch). It runs three jobs:

1. **detect** — `dorny/paths-filter` sets booleans for `frontend`,
   `backend`, and `migrations`. The downstream jobs gate on these.

2. **deploy-frontend** — if `frontend/**` changed:
   - `pnpm install && pnpm build`
   - `npx netlify-cli deploy --prod` with `NETLIFY_AUTH_TOKEN` +
     `NETLIFY_SITE_ID`.

3. **deploy-backend** — if `backend/**` or `infra/**` changed:
   - `cargo build --release --bin chakramcp-server` natively on
     the ubuntu-22.04 runner (no cross-compile — runner IS x86_64).
   - `cp target/release/chakramcp-server infra/chakramcp-server`.
   - `docker build -f infra/Dockerfile.thin` → `docker push` to
     `877326604850.dkr.ecr.us-east-1.amazonaws.com/chakramcp-server`
     with tags `${sha:0:7}` + `latest`.
   - **If migrations changed** (`backend/migrations/**`):
     `docker compose --profile migrate run --rm migrate` over SSH
     to `ubuntu@54.84.88.246` (Lightsail prod). Runs BEFORE the
     relay restart so new code never sees an old schema.
   - `docker compose up -d --force-recreate relay`.
   - Probe `https://relay.chakramcp.com/healthz`, `/readyz`,
     `/v1/discovery/agents` — fail the workflow if any return non-200.

### Required secrets

Set once via `gh secret set <NAME> --repo Delta-S-Labs/chakra_mcp`:

| Secret | What | How to get it |
|---|---|---|
| `NETLIFY_AUTH_TOKEN` | Personal token for `netlify deploy` | Netlify → User settings → Applications → New token |
| `NETLIFY_SITE_ID` | `ef540682-67b3-46e6-a425-afbe85437f88` | Already known; just set the value |
| `AWS_ACCESS_KEY_ID` | IAM user creds for ECR push | Create user `cd-publisher` with `AmazonEC2ContainerRegistryPowerUser` policy |
| `AWS_SECRET_ACCESS_KEY` | Pair of above | Same user |
| `LIGHTSAIL_SSH_KEY` | Private key for `ubuntu@54.84.88.246` | Contents of `~/.ssh/lightsail-chakramcp-prod.pem` |

OIDC trust to AWS is the production upgrade path — replaces the
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
```

Useful when:

- You want to redeploy without a code change (e.g. flip env vars
  and want them in the bundle).
- A deploy failed mid-step and you fixed the env without
  triggering a re-merge.

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

- #11 `@types/node` 20→25 — major-version bump for dev-deps, will hold
- #12 `next` group bump — patch/minor; auto-merge after rebase
- #13 `react` group bump — likely patch; auto-merge after rebase

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
