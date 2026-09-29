# Self-hosting, Phase 2: public image, Compose and Kubernetes packaging

**Status:** design, awaiting review
**Date:** 2026-09-29
**Builds on:** `2026-09-29-observability-phase1-design.md` (production observability, shipped in #339, #340, #348, #349).

## 1. Goal

An open-source self-hoster can run ChakraMCP, with metrics, logs, dashboards and alerts, from public artifacts:
- with **Docker Compose**, using the same files production runs;
- on **Kubernetes**, with a Helm chart.

Today self-hosters have only `brew install chakramcp-server`, a source build, or `infra/Dockerfile.thin`. There is no public container image and no Kubernetes support. The Phase 1 observability files hardcode our domains, GitHub sign-in and Telegram.

**Decided with the user (2026-09-29):**

| Question | Decision |
|---|---|
| What Kubernetes should cover | ChakraMCP itself **and** observability |
| Postgres and Redis in the chart | Bundled simple ones (official images) for evaluation and small installs, plus external services for production. No Bitnami: its free images were withdrawn in 2025. |
| Observability on Kubernetes | Always ship integration pieces for an existing Prometheus/Grafana; an optional `bundled` switch installs a stack |
| Publishing | Tagged releases **and** an `:edge` image from every push to main |
| Grafana sign-in and alerts for self-hosters | Settings-driven: admin login by default; GitHub, Google or OIDC plus an allowlist when configured; alert channel chosen by a setting |

**Out of scope:**
- autoscaling, Postgres backups and HA (external Postgres covers these);
- PrometheusRule/Alertmanager versions of the alerts (Grafana-managed rules only);
- image signing;
- changing production's private ECR image pipeline;
- Windows hosts.

## 2. Principles

- **Production runs the self-host files.** The Phase 1 Compose files are generalized, not copied. Everything specific to chakramcp.com moves into the VM's `.env`. If the open-source path breaks, production breaks too, so it can't rot quietly.
- **One source for dashboards and alert rules.** They stay in `infra/observability/grafana/`. The chart gets synced (and, for rules, filtered) copies, and CI fails if a copy drifts.
- **Every PR is safe on its own** and leaves production behaving exactly as before.

## 3. Public image

**Name:** `ghcr.io/delta-s-labs/chakramcp-server`, for `linux/amd64` and `linux/arm64`.

**Tags:**
- On every push to main that touches `backend/**`, `infra/Dockerfile.release` or `image.yml`: `:edge` and `:sha-<7>`.
- On a release `X.Y.Z`: `:X.Y.Z`, plus `:X.Y` and `:latest` **only when the version has no pre-release suffix**, so e.g. `0.2.0-rc.1` gets only its own tag.

**`infra/Dockerfile.release` (new):**
- Base `debian:bookworm-slim` with only `ca-certificates` and `wget` (for the healthcheck). There's no `libssl3`: the backend has no OpenSSL dependency and uses rustls.
- User `chakra` (uid 10001); ports 8080/8090, plus 9464 for metrics; `ENTRYPOINT ["chakramcp-server"]`, `CMD ["start"]`; OCI labels (source, version, revision, licenses).
- It copies a **prebuilt binary** from the build context (`dist/chakramcp-server`).
- `Dockerfile.thin` and production's ECR build stay as they are.

**No emulation: each architecture is built on its own runner.**
- Each job builds the binary and a single-platform image, then pushes the image **by digest**.
- A final job joins the two digests into one multi-arch tag with `docker buildx imagetools create`.
- The runners are `ubuntu-22.04` (amd64) and `ubuntu-22.04-arm` (arm64). Their glibc 2.35 is older than bookworm's 2.36, and they match `cd.yml` and `cli-release.yml`.
- Images carry build provenance (`provenance: mode=max`) and an SBOM.
- Permissions: `packages: write`.

**`.github/workflows/image.yml` (new, `:edge`):**
- Triggers on the paths above.
- `concurrency: image-edge`, `cancel-in-progress: true`. A superseded run never reaches the merge step, so an older build can't overwrite `:edge`.
- The binary is built with `SQLX_OFFLINE=true`, `--release --locked`, `GIT_SHA=<7>` and `CHAKRAMCP_VERSION=edge`.

**Releases (`cli-release.yml`, tag `cli-vX.Y.Z`).** One release versions the CLI, the server, the image and the chart together.
- **Build step:** the existing server build step also exports `GIT_SHA` and `CHAKRAMCP_VERSION=X.Y.Z`. Both are compile-time, so without them release binaries would report `0.1.0` / `unknown`.
- **New `image` job:**
  - `needs: [resolve-version, build]`, with job-level `packages: write`;
  - a matrix over `x86_64-unknown-linux-gnu`/amd64 and `aarch64-unknown-linux-gnu`/arm64, each on its native runner;
  - each leg downloads its target's artifact, unpacks `chakramcp-server-X.Y.Z-<triple>.tar.gz` into `dist/`, and pushes its digest;
  - a merge job applies the tags above.
- **New `chart` job:** added in PR-3 (§5.6).

**Version reporting.** `chakramcp_build_info{version}` reports `CHAKRAMCP_VERSION` and falls back to the crate version. This is a one-line change in the server's `BuildInfo`.

**One-time manual step.** After each package's first push (the image, and later the chart), make it public in the GHCR package settings if it didn't inherit the repo's visibility.

## 4. Compose: the Phase 1 files, generalized

The same files serve production and self-hosters, and they keep their names: `infra/docker-compose.prod.yml` (which CD copies to `docker-compose.yml` on the VM), `infra/Caddyfile` and `infra/observability/`. Every new setting defaults to something that works for a self-hoster. Production's current behaviour moves into the VM's `.env` **before** the PR merges (§8).

**Scripts.** `deploy.sh` and `smoke-test.py` take the base file name from `COMPOSE_BASE`, defaulting to `docker-compose.yml` as on the VM. Self-hosters running from a checkout's `infra/` set `COMPOSE_BASE=docker-compose.prod.yml`.

### 4.1 Base (`infra/docker-compose.prod.yml`, `infra/Caddyfile`, new `infra/.env.example`)

- **Image.** `image: ${CHAKRAMCP_IMAGE:-ghcr.io/delta-s-labs/chakramcp-server:latest}` for `relay` and `migrate`. Production sets its ECR image, as today, and CD still pins the exact sha.
- **Log driver.** `x-logging` uses `driver: ${LOG_DRIVER:-journald}`, which needs a host with systemd's journald, as production has.
  - On Docker Desktop or hosts without journald, `LOG_DRIVER=json-file` makes the base services start. Logs then stay out of Loki, since Alloy reads the journal.
  - The `mode` and `labels` options work with both drivers.
- **Caddy hostnames come from the environment.** The `caddy` service's environment gets:
  - `APP_DOMAIN: ${APP_DOMAIN:?}` and `RELAY_DOMAIN: ${RELAY_DOMAIN:?}` (required);
  - `GRAFANA_DOMAIN: ${GRAFANA_DOMAIN:-localhost}`.

  The defaults live in Compose because Caddy's `{$VAR:default}` doesn't apply to a variable that is set but empty. The Caddyfile uses plain `{$APP_DOMAIN}`, `{$RELAY_DOMAIN}` and `{$GRAFANA_DOMAIN}`. With the `localhost` default, Caddy serves the Grafana site with a local certificate and no ACME, which is harmless when observability is off.
- **The `email` global option is removed.** It would be a parse error when empty, ACME works without it, and Let's Encrypt no longer sends expiry emails. Production loses nothing.
- **CD applies Caddy changes safely.** Today's step reloads the running container, which would lack the new environment.
  - The backend job now runs `docker compose up -d --no-deps caddy` after syncing. This recreates Caddy only when its definition changed, for example the new env; otherwise it's a no-op.
  - It then keeps the hash-gated `caddy reload` for Caddyfile-only changes.
- **`infra/.env.example`** documents every key, grouped: required, sign-in providers, limits, observability, `LOG_DRIVER`. It uses placeholders only.

### 4.2 Observability overlay (`infra/observability/`)

**Grafana sign-in.** Every provider is off unless configured; the defaults give admin login only.

| Setting (`.env`) | Default | Maps to |
|---|---|---|
| `GRAFANA_DOMAIN` | `localhost` | `GF_SERVER_ROOT_URL=https://$GRAFANA_DOMAIN` |
| `GRAFANA_DISABLE_LOGIN_FORM` | `false` | `GF_AUTH_DISABLE_LOGIN_FORM` |
| `GRAFANA_GITHUB_ENABLED` / `_CLIENT_ID` / `_CLIENT_SECRET` / `_AUTO_LOGIN` | off | `GF_AUTH_GITHUB_*` (scopes `user:email,read:org`; Grafana needs `read:org` at every sign-in) |
| `GRAFANA_GOOGLE_ENABLED` / `_CLIENT_ID` / `_CLIENT_SECRET` | off | `GF_AUTH_GOOGLE_*` |
| `GRAFANA_OIDC_ENABLED` / `_NAME` / `_CLIENT_ID` / `_CLIENT_SECRET` / `_AUTH_URL` / `_TOKEN_URL` / `_API_URL` / `_SCOPES` | off; scopes `openid email profile` | `GF_AUTH_GENERIC_OAUTH_*` |
| `GRAFANA_ROLE_ATTRIBUTE_PATH` | empty | `role_attribute_path` for each enabled provider |

- Every provider gets `role_attribute_strict=true` and `allow_assign_grafana_admin=true`.
- The allowlist expression is evaluated against **each provider's own user info**: GitHub's `/user` (e.g. `id`, `login`), Google's and OIDC's claims (e.g. `email`, `sub`). The docs give an example for each.
- An empty allowlist refuses everyone for that provider (fail closed).
- `GRAFANA_ADMIN_PASSWORD` stays required.
- The Caddy rule refusing `Basic` auth from outside stays: the admin login uses the form and a cookie.

**Alert delivery: `ALERT_CHANNEL`** is one of `none` (the default), `telegram`, `slack`, `email` or `webhook`.
- **Channel files.** Each channel is a file in `infra/observability/grafana/channels/`:
  - its contact point, under a fixed uid (`chakramcp-telegram`, `chakramcp-slack`, …), with settings read from env: `TELEGRAM_BOT_TOKEN`/`TELEGRAM_CHAT_ID`, `SLACK_WEBHOOK_URL`, `ALERT_EMAIL_TO` plus `GF_SMTP_*`, `ALERT_WEBHOOK_URL`;
  - the root policy pointing at it;
  - `deleteContactPoints` for the **other** channels' uids, so switching channels removes the old one (file provisioning never deletes on its own).
- **`none.yml`** deletes all the channel contact points and runs `resetPolicies: [1]`. Alerts then show in Grafana only.
- **Mounting.** Grafana reads only the top level of `provisioning/alerting` (verified in its source). A committed placeholder `provisioning/alerting/channel.yml` (`apiVersion: 1`) gives the single-file mount a real target inside the read-only provisioning mount. Compose mounts `channels/${ALERT_CHANNEL:-none}.yml` over it.
- **Template.** The message template moves to `provisioning/alerting/templates.yml`, so all channels share it.
- **Deploys.**
  - CD's hash loop adds `grafana/channels`, and `deploy.sh` recreates Grafana when it changes, because a single-file mount doesn't see rsync's replaced file.
  - Changing `ALERT_CHANNEL` changes the mount source, so Compose recreates Grafana by itself.
- **Production** sets `ALERT_CHANNEL=telegram`. Its current `notifications.yml` becomes `channels/telegram.yml`, with the same uid, settings, template and policy.

**HTTPS probes: `PROBE_TARGETS`**, a JSON list such as `[{"name":"relay","address":"https://relay.example.com/healthz"}]`.
- Alloy reads it with `encoding.from_json(coalesce(sys.env("PROBE_TARGETS"), "[]"))` into `prometheus.exporter.blackbox`'s `targets`.
- The default is no probes. The implementation confirms that Alloy accepts an empty list; if it doesn't, the probe component moves to a separate file that's only included when the list isn't empty.
- **Probes are HTTPS-only** (`fail_if_not_ssl`).
- The "Backend down" rule then relies on the metrics scrape alone, which is its existing `or` branch. The rules look for the probe names `relay` and `app`, and `.env.example` says so.
- Production sets its four current targets.
- **The Overview dashboard's** four fixed probe tiles become **one stat panel over every probe** (one tile per probe name), so any set of probes displays, and none shows as one "No data" tile.

### 4.3 First start and upgrades (self-hosters)

From a checkout's `infra/`:
1. `cp .env.example .env` and fill it in.
2. `docker compose -f docker-compose.prod.yml up -d`.
3. `COMPOSE_BASE=docker-compose.prod.yml observability/scripts/deploy.sh "alloy prometheus loki grafana/provisioning grafana/dashboards grafana/channels"`, the same script CD runs. It creates the Postgres monitoring role, brings the stack up, and verifies the configs loaded.

Upgrades: `git pull`, then the same two commands. `docs/self-hosting/compose.md` walks through it.

### 4.4 CI

**`observability-ci.yml`:**
- The static checks run with `ci.env` extended for the new keys:
  - `caddy validate` gets `APP_DOMAIN`, `RELAY_DOMAIN` and `GRAFANA_DOMAIN`;
  - the Compose config is checked with `ALERT_CHANNEL=none` and with `telegram`.
- The smoke test also starts the **relay** from `:edge` and asserts:
  - `up{job="chakramcp"} == 1`;
  - a `chakramcp_build_info` series;
  - JSON relay logs in Loki with a `level` label.

## 5. Helm chart: `charts/chakramcp`

### 5.1 Workload

- **The server Deployment:**
  - pods labelled `app.kubernetes.io/component: relay`, matching Compose's service name, which the dashboards and rules key on;
  - ports `app` 8080, `relay` 8090 and `metrics` 9464 (the metrics port only when `metrics.enabled`, the default);
  - liveness probe `/healthz`, readiness probe `/readyz`;
  - `runAsNonRoot` (uid 10001), `readOnlyRootFilesystem`, `capabilities.drop: [ALL]`, `seccompProfile: RuntimeDefault`;
  - `replicaCount` defaults to 1. More replicas are safe: migrations take an advisory lock, the credits worker holds `pg_try_advisory_xact_lock` for accounting, and rate-limit counters live in Redis.
- **Services:** one ClusterIP Service with the `app`, `relay` and `metrics` ports.
- **Ingress** (optional; `className`, `annotations` such as cert-manager, `tls`) has two hosts: `hosts.app` → 8080 and `hosts.relay` → 8090.
- **Configuration:**
  - **Env:** `LOG_FORMAT=json`, `METRICS_ADDR=0.0.0.0:9464`, public URLs, `ADMIN_EMAIL`, `LIMITS_ENFORCE`, `DISCOVERY_V2`, plus an `extraEnv` escape hatch.
  - **Secrets:** `JWT_SECRET`, `WEBHOOK_SIGNING_SECRET` and the optional `UPSERT_SHARED_SECRET` come from `secrets.existingSecret`. Otherwise the chart generates a Secret on first install and keeps it across upgrades, using `lookup`.
- **Migrations** run at boot, as today. There is no hook Job.
- **Rollbacks.** The docs warn that `helm rollback` across a release that added a migration fails, because the server refuses to boot on an unknown applied migration: roll forward instead.

### 5.2 Postgres and Redis

| | Bundled (default) | External |
|---|---|---|
| Postgres | `postgresql.enabled=true`: a StatefulSet from `postgres:16-alpine` with a PVC (`persistence.size`, `storageClass`). The password comes from `postgresql.auth.existingSecret`/`key`, or is generated and kept with `lookup`. | `postgresql.enabled=false` plus `externalDatabase.url`, or `existingSecret`/`key` |
| Redis | `redis.enabled=true`: a Deployment from `redis:7-alpine` with Phase 1's flags (no persistence, `maxmemory` + LRU) | `redis.enabled=false` plus `externalRedis.url`. Unset means rate limiting fails open, as today. |

- **GitOps.** `helm template`/Argo can't `lookup`, so every generated value would change on each render. The docs and `NOTES.txt` say GitOps users must set **both** `secrets.existingSecret` and `postgresql.auth.existingSecret`.
- The bundled Postgres is labelled "for evaluation and small installs". The chart README recommends a managed or operator-run Postgres for production.

### 5.3 Observability: integration (on by default)

**Scraping.** A `ServiceMonitor`, rendered only when the `monitoring.coreos.com/v1` API exists, scrapes the metrics port and relabels `job` to `chakramcp`.
- Its labels come from `observability.serviceMonitor.labels`.
- The docs spell out that kube-prometheus-stack only picks up ServiceMonitors carrying its `release: <name>` label (`serviceMonitorSelectorNilUsesHelmValues`).

**Dashboards.** ConfigMaps labelled `grafana_dashboard: "1"`, which kube-prometheus-stack's sidecar picks up in all namespaces:
- **Overview**, always;
- **Logs**, only with `observability.loki.enabled=true`: it needs a Loki with the `service`/`level` labels.
- The **Infrastructure** dashboard (host, Caddy, Compose services) stays Compose-only; on Kubernetes the cluster's own dashboards cover nodes and pods.

**Alert rules.** A ConfigMap labelled `grafana_alert: "1"`. This is **an explicit rule set, by uid**; the sync filters the source rules to it, and the drift check compares the filtered result.

| Rule uid | On Kubernetes |
|---|---|
| `backend-down` | yes (its scrape branch works; the probe branch is `or`'d and simply absent) |
| `high-5xx-ratio`, `credit-switches-stale`, `credit-queue-backlog` | yes |
| `error-log-spike` | only with `observability.loki.enabled` |
| `postgres-down`, `certificate-expiring`, `disk-filling`, `memory-low`, `oom-kill`, `crash-loop`, `monitoring-blind` | no: they need the Compose exporters or probes, and would fire permanently without them |

- **The alerts sidecar.** kube-prometheus-stack's Grafana has its alerts sidecar **off** by default, and it searches only its own namespace. The docs say to set `grafana.sidecar.alerts.enabled=true` (with `searchNamespace: ALL`), or to put the ConfigMap in Grafana's namespace with `observability.alertRules.namespace`.
- **Datasource UIDs** are values (`observability.datasources.prometheus`, default `prometheus` like kube-prometheus-stack; `.loki`, default `loki`), substituted into the synced JSON and YAML at render time.
- **Asset sync.** `infra/observability/scripts/sync-chart-assets.sh` writes `charts/chakramcp/files/`: the Overview and Logs dashboards, and the filtered rules. CI re-runs it and fails on any difference.

### 5.4 Observability: bundled (off by default; `observability.bundled.enabled=true`)

**Subcharts** (Chart.yaml dependencies, pinned, conditional on the switch), from their **current** homes:

| Component | Chart | Settings |
|---|---|---|
| Prometheus | `prometheus-community/prometheus` | server only, with Alertmanager, pushgateway, node-exporter and kube-state-metrics off; `--web.enable-remote-write-receiver`; 15-day retention |
| Loki | the community-maintained Loki chart (moved in March 2026, alongside Grafana; the exact repo is verified at implementation) | single binary, filesystem, 14-day retention; memcached caches, gateway and canary off |
| Grafana | `oci://ghcr.io/grafana-community/helm-charts` → `grafana` (the old `grafana/helm-charts` copy is frozen since January 2026) | datasources with the UIDs above; dashboard **and alert** sidecars on; sign-in and alert channel values mirroring §4.2 (`grafana.ini` auth sections; the channel's contact point from a Secret) |
| Alloy | `grafana/alloy` | **one replica as a Deployment**, clustering off, with the chart's config (below) |

**Alloy's Kubernetes config** is its own small file, since discovery differs from Compose:
- **Metrics:** `discovery.kubernetes` finds the chart's pods by release labels, scrapes their metrics port as `job="chakramcp"`, and remote-writes to the bundled Prometheus. It's the only scraper, because there's no Prometheus Operator in bundled mode, so the ServiceMonitor doesn't apply.
- **Logs:** `loki.source.kubernetes` tails the release namespace's pods through the API. It sets `service` from `app.kubernetes.io/component` and applies the same JSON `level` extraction as Compose.
- **One replica:** with a DaemonSet, every pod would tail every pod's logs and multiply them by the node count.
- In bundled mode, `observability.loki.enabled` is implied.

### 5.5 Values and schema

`values.schema.json` validates the values:
- required URLs when an Ingress is on;
- the external database URL when bundled Postgres is off;
- enum checks, e.g. the alert channel.

`NOTES.txt` prints how to reach the app and relay, where the generated secrets are, the GitOps warning, and next steps. A `helm test` pod curls `/healthz` and `/metrics`.

### 5.6 Publishing

`cli-release.yml` gains a `chart` job in PR-3. It needs `resolve-version` and the image job, and has job-level `packages: write`. It runs:
- `helm dependency build`;
- `helm package charts/chakramcp --version X.Y.Z --app-version X.Y.Z`;
- `helm push` to `oci://ghcr.io/delta-s-labs/charts`.

No `:edge` chart is published; unreleased chart changes install from a checkout.

### 5.7 CI: `chart-ci.yml` (new)

Runs on PRs and path-gated (`charts/**`, the synced sources, the workflow):
- **Static:**
  - `helm lint`;
  - `helm template` for each values file in `charts/chakramcp/ci/` and `charts/chakramcp/ci-template/`, the latter with `--api-versions monitoring.coreos.com/v1/ServiceMonitor` where the ServiceMonitor is under test;
  - `kubeconform -strict` against the current Kubernetes release and one older one, with CRD schemas;
  - the asset-sync check.
- **Install on `kind`** with `ct install`, using the `:edge` image. `ct` installs every file in `ci/`, so only installable variants live there; template-only variants such as external DB or CRDs live in `ci-template/`.
  - `ci/default-values.yaml`: wait for readiness, then `helm test`;
  - `ci/bundled-values.yaml`: assert that Grafana is up, the dashboards and rules are provisioned, and Prometheus has `up{job="chakramcp"} == 1`.

Once it's stable, it's added to the required checks (ask the user then).

## 6. Documentation

- **`docs/self-hosting/`:**
  - `README.md`: choose Compose or Kubernetes, and the host requirements (Linux with journald for the default Compose logging);
  - `compose.md`: the §4.3 walkthrough, DNS and TLS, upgrades;
  - `kubernetes.md`: `helm install`, bundled vs external databases, Ingress/TLS, secrets and GitOps, kube-prometheus-stack integration, upgrades and rollbacks;
  - `observability.md`: sign-in providers and allowlist examples per provider, alert channels, probes, what each dashboard and alert means, and which apply on Kubernetes.
- **Other docs:** `docs/INSTALL.md` gains rows for the image and the chart. `docs/CI-CD.md` describes `image.yml`, `chart-ci.yml`, the release additions and the CD Caddy change.

## 7. Testing summary

| Layer | How |
|---|---|
| Image | Per-arch builds; after the merge job, pull `:sha-<7>` on both runners and run `chakramcp-server --help`. |
| Compose (generic and production settings) | `observability-ci.yml`: config checks for both channel settings, and a smoke test with the relay on `:edge` |
| Chart | `chart-ci.yml`: lint, template (with and without the CRDs), kubeconform, kind install (defaults and bundled), `helm test` |
| Production (PR-2) | **Before merge:** render the new files with the production `.env` on the VM (`docker compose config`, `caddy adapt`) and compare with today's effective config. **After merge:** the Phase 1 §10.4 checklist, plus a contact-point test (with the user's OK). |

## 8. Rollout (PRs)

1. **PR-1, the public image:** `Dockerfile.release`, `image.yml`, the `CHAKRAMCP_VERSION` build info, and in `cli-release.yml` the build-step env plus the `image` job; `INSTALL.md` (image) and `CI-CD.md` (image).
   - **After merge:** confirm `:edge` exists and is public, and pull it on both architectures.
2. **PR-2, the Compose generalization (§4):**
   - **Contents:**
     - the base and overlay changes;
     - `.env.example`;
     - the `cd.yml` Caddy recreate step and the `grafana/channels` hash;
     - `COMPOSE_BASE` in the scripts;
     - the probe stat panel;
     - the `observability-ci` updates;
     - `docs/self-hosting/README.md`, `compose.md` and `observability.md` (Compose parts), and `INSTALL.md`.
   - **Before merge:**
     - add the production keys to the VM's `.env`: `APP_DOMAIN`, `RELAY_DOMAIN`, `GRAFANA_DOMAIN`, `GRAFANA_GITHUB_ENABLED=true`, `GRAFANA_GITHUB_AUTO_LOGIN=true`, `GRAFANA_DISABLE_LOGIN_FORM=true`, `ALERT_CHANNEL=telegram`, `PROBE_TARGETS=[…]`;
     - run the dry run in §7.
   - **After merge:** CD recreates Caddy once with its new environment, a brief blip. Then run the §7 checks.
3. **PR-3, the chart core:** §5.1, §5.2 and §5.5, `chart-ci.yml` (default install), the `cli-release.yml` `chart` job, and `docs/self-hosting/kubernetes.md`.
4. **PR-4, chart observability:** §5.3 and §5.4, the asset sync, the bundled kind test, a Dependabot `helm` entry for the pinned subcharts, and the Kubernetes parts of `observability.md`.
5. **The first versioned release:** cut the tag (ask the user for the version), then check the versioned image and the OCI chart, and make the chart package public.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Generalizing Compose changes production's behaviour | Production's values go into `.env` *before* the merge; a dry run on the VM compares the rendered config; CI tests both channel settings; post-merge checks |
| CD applies an env-driven Caddyfile to a container without that env | CD recreates Caddy when its definition changes (`up -d --no-deps caddy`) before the hash-gated reload |
| The GHCR packages stay private | A one-time manual toggle per package, verified after the first push |
| Old glibc or emulation problems in the image | Native builds on `ubuntu-22.04`/`-arm` (glibc 2.35 ≤ bookworm 2.36), merged by digest; no QEMU |
| Subchart sources move again or break | Current homes pinned in Chart.yaml; Dependabot's `helm` ecosystem; the bundled kind test |
| Generated secrets change under GitOps | `existingSecret` for the app secrets **and** the Postgres password, required for GitOps and documented |
| An existing kube-prometheus-stack ignores the chart's pieces | ServiceMonitor labels are a setting; the docs cover the `release` label and enabling the alerts sidecar |
| Kubernetes alerts that can never be satisfied | An explicit Kubernetes rule set; the Loki-dependent pieces are gated |
| Compose on hosts without journald | `LOG_DRIVER=json-file`, documented as a host requirement choice |
| Bundled Postgres used as production storage | Labelled for evaluation; the docs recommend external Postgres |
