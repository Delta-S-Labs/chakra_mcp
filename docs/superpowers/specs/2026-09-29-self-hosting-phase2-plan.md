# Self-hosting, Phase 2 — Implementation Plan

Derived from `2026-09-29-self-hosting-phase2-design.md` (approved 2026-09-29). § numbers refer to that spec. The work ships as five PRs in order; each leaves production unchanged unless stated.

**Conventions**
- Same as Phase 1: the commit trailer and PR footer, and branch → PR → green CI → squash `--admin` → verify *that commit's* CD run.
- Never print a secret. Read `.env` keys with `grep`/`cut`; never `source` it.
- Workflow changes pass `actionlint`, and shell passes `shellcheck` (both run in Docker locally).
- Local Docker is colima: bind mounts only work under `$HOME`, and pulls go through a temporary `DOCKER_CONFIG` that has no credential helper.

---

## PR-1 — public image (§3)

1. **Build info.** `backend/server/src/main.rs`: `BuildInfo.version` = `option_env!("CHAKRAMCP_VERSION")`, else `CARGO_PKG_VERSION`. Apply the same one-liner in `app`/`relay` `main.rs`.
2. **`infra/Dockerfile.release`:**
   - `debian:bookworm-slim` + `ca-certificates` + `wget`; user 10001; ports 8080/8090/9464;
   - the HEALTHCHECK as in `Dockerfile.thin`;
   - `COPY dist/chakramcp-server /usr/local/bin/`;
   - OCI labels through `ARG`s (version, revision, created).
3. **`.github/workflows/image.yml` (edge):**
   - **Triggers and concurrency:** `on: push: branches [main], paths [backend/**, infra/Dockerfile.release, .github/workflows/image.yml]` plus `workflow_dispatch`; `concurrency: {group: image-edge, cancel-in-progress: true}`; `permissions: {contents: read, packages: write}`.
   - **Job `build` (matrix):** `{arch: amd64, runner: ubuntu-22.04, target: x86_64-unknown-linux-gnu}` and `{arch: arm64, runner: ubuntu-22.04-arm, target: aarch64-unknown-linux-gnu}`. Each leg:
     - sets up the rust toolchain and `Swatinem/rust-cache` (key `image-<arch>`);
     - runs `SQLX_OFFLINE=true GIT_SHA=<7> CHAKRAMCP_VERSION=edge cargo build --release --locked --bin chakramcp-server`;
     - copies the binary to `dist/`;
     - runs `docker/setup-buildx-action` and `docker/login-action` (ghcr, `GITHUB_TOKEN`);
     - runs `docker/build-push-action` with `platforms: linux/<arch>`, `outputs: type=image,name=ghcr.io/delta-s-labs/chakramcp-server,push-by-digest=true,name-canonical=true,push=true`, `provenance: mode=max`, `sbom: true`, and the labels;
     - uploads the digest as an artifact.
   - **Job `merge`:**
     - downloads the digests;
     - `docker buildx imagetools create -t …:edge -t …:sha-<7> <digests…>`;
     - `docker buildx imagetools inspect …:edge` shows both platforms.
   - **Job `verify` (matrix, both runners):** `docker run --rm ghcr.io/…:sha-<7> --help` succeeds, and `docker run … --version`, or the build info, reports `edge`.
4. **`cli-release.yml`:**
   - **Build step:** the server build step's run line becomes `GIT_SHA="${GITHUB_SHA:0:7}" CHAKRAMCP_VERSION="${{ needs.resolve-version.outputs.version }}" $CARGO build -p chakramcp-server …`. The CLI build is unchanged.
   - **New job `image`:** `needs: [resolve-version, build]`, `permissions: {contents: read, packages: write}`, with the same matrix of 2 legs. Each leg:
     - downloads the artifact named `<target>`;
     - runs `tar -xzf chakramcp-server-<ver>-<target>.tar.gz -C dist`;
     - does the same per-arch push-by-digest as `image.yml`.
   - **New job `image-merge`:**
     - tags `:<ver>`, plus `:<X.Y>` and `:latest` only if `<ver>` has no `-`;
     - checks with `imagetools inspect`.
5. **Docs.** `docs/INSTALL.md` gains a "Container image" row with the pull command and tags. `docs/CI-CD.md` gains an "Image" section covering `image.yml`, the release jobs and the one-time public toggle.
6. **Verify locally:**
   - `cargo test --workspace` and `clippy`;
   - build `Dockerfile.release` locally (amd64, via colima) with a locally built Linux binary, or skip this and rely on CI;
   - `actionlint`.
7. **Ship:**
   1. Open the PR; CI green; merge.
   2. Watch `image.yml` on main.
   3. **Manual:** make the package public (GitHub → Packages → chakramcp-server → settings); do this with the user's OK.
   4. `docker pull ghcr.io/delta-s-labs/chakramcp-server:edge` anonymously, on both architectures (the verify job).

## PR-2 — Compose generalization (§4)

1. **Base compose (`infra/docker-compose.prod.yml`):**
   - `x-logging.driver: ${LOG_DRIVER:-journald}`;
   - the `relay`/`migrate` image default to GHCR `:edge` (it becomes `:latest` in PR-5);
   - `caddy.environment`: `APP_DOMAIN: ${APP_DOMAIN:?…}`, `RELAY_DOMAIN: ${RELAY_DOMAIN:?…}`, `GRAFANA_DOMAIN: ${GRAFANA_DOMAIN:-localhost}`;
   - the header comment explains the env.
2. **`infra/Caddyfile`:**
   - site addresses become `{$APP_DOMAIN}`, `{$RELAY_DOMAIN}` and `{$GRAFANA_DOMAIN}`;
   - the `email` global option is removed.
3. **`infra/.env.example` (new):** every key used by the base and overlay, grouped, with placeholders and comments. It includes `LOG_DRIVER`, `COMPOSE_BASE` (a note), the provider settings, `ALERT_CHANNEL`, `PROBE_TARGETS` (probe names `relay` and `app` are the ones the rules use), and SMTP for email.
4. **Overlay `infra/observability/compose.yml`, Grafana env:**
   - Map the §4.2 table: GitHub, Google, generic OAuth; each has `ROLE_ATTRIBUTE_PATH=${GRAFANA_ROLE_ATTRIBUTE_PATH:-}`, `ROLE_ATTRIBUTE_STRICT=true` and `ALLOW_ASSIGN_GRAFANA_ADMIN=true`.
   - Login form: `${GRAFANA_DISABLE_LOGIN_FORM:-false}`; root URL: `https://${GRAFANA_DOMAIN:-localhost}`.
   - Drop the `:?` requirements on GitHub, Telegram and the role path. `GRAFANA_ADMIN_PASSWORD` and `PG_MONITOR_PASSWORD` stay required.
   - Channel env passthrough: `TELEGRAM_*`, `SLACK_WEBHOOK_URL`, `ALERT_EMAIL_TO`, `ALERT_WEBHOOK_URL`, `GF_SMTP_*`.
   - Volumes: add `./observability/grafana/channels/${ALERT_CHANNEL:-none}.yml:/etc/grafana/provisioning/alerting/channel.yml:ro`.
5. **Alerting files:**
   - `provisioning/alerting/templates.yml`: the shared template, moved from `notifications.yml`.
   - `provisioning/alerting/channel.yml`: the placeholder, `apiVersion: 1`.
   - `grafana/channels/{none,telegram,slack,email,webhook}.yml`: each has contact point uid `<channel>-chakramcp` (so telegram keeps production's `telegram-chakramcp` and its name `telegram`), the root policy, and `deleteContactPoints` for the other uids. `none` deletes all and uses `resetPolicies: [1]`.
   - Delete `notifications.yml`.
6. **Alloy:**
   - `prometheus.exporter.blackbox` uses `targets = encoding.from_json(coalesce(sys.env("PROBE_TARGETS"), "[]"))`, and the overlay passes `PROBE_TARGETS: ${PROBE_TARGETS:-[]}`.
   - **Verify** Alloy accepts `[]` with `alloy run` locally. If not, split the probe components into `alloy/probes.alloy`, loaded only when set; Alloy loads a directory, so `--config` becomes the directory.
7. **Scripts:**
   - `deploy.sh` and `smoke-test.py` pick `docker-compose.yml` when present, else `docker-compose.prod.yml`; `COMPOSE_BASE` overrides. `deploy.sh` calls `ensure-monitor-role.sh -f "$COMPOSE_BASE" -f observability/compose.yml`.
   - **Journald preflight (§4.1).** The overlay's `x-logging.driver` reads `${LOG_DRIVER:-journald}`. `deploy.sh` first checks `LOG_DRIVER`, then `/etc/machine-id` and `/var/log/journal` on the Docker host (a throwaway container with strict mounts), and exits with the fix before running Compose. The paths are overridable variables so a test can point them at missing files. Alloy's `/etc/machine-id` and `/var/log/journal` mounts switch to the long syntax with `create_host_path: false`. Test that the preflight fails on missing paths and on `LOG_DRIVER=json-file`, and that the stack still starts on colima, which has all three paths.
   - `deploy.sh` recreates Grafana when `grafana/channels` changed.
   - `check-grafana.py` also parses `channels/*.yml`.
8. **Overview dashboard.** Edit `overview.json` directly: Phase 1's generator was a scratch tool and isn't in the repo. Replace the four probe stat tiles with one stat panel: `label_replace(probe_success, "probe", "$1", "job", "integrations/blackbox/(.*)")`, legend `{{probe}}`, with UP/DOWN mappings. `check-grafana.py` validates it; the diff touches only those panels.
9. **`cd.yml`:**
   - In the backend job, after "Sync infra config", add the step `docker compose up -d --no-deps caddy`. It prints whether Caddy was recreated.
   - The existing reload stays hash-gated.
   - The observability job's hash loop adds `grafana/channels`.
10. **`observability-ci.yml`:**
    - `ci.env` gains the new keys (domains, `ALERT_CHANNEL=none`, `PROBE_TARGETS=[]`).
    - Compose config runs with `ALERT_CHANNEL=none` and `=telegram`.
    - `caddy validate` gets `-e APP_DOMAIN=… -e RELAY_DOMAIN=… -e GRAFANA_DOMAIN=…`.
    - The smoke test also starts `relay` with `CHAKRAMCP_IMAGE=ghcr.io/delta-s-labs/chakramcp-server:edge` (plus the env the relay needs, from `ci.env`).
    - Assert `up{job="chakramcp"}`, `chakramcp_build_info`, and relay logs with `level`.
    - `smoke-test.py`'s contact-point check follows `ALERT_CHANNEL`: with `none`, no `*-chakramcp` contact point; otherwise the channel's uid.
    - Confirm the runner has `/var/log/journal`, which the strict mount now needs; if it doesn't, the workflow creates it before starting the stack.
11. **Docs:**
    - `docs/self-hosting/README.md`, `compose.md` (the §4.3 walkthrough), and `observability.md` (Compose parts: providers with an allowlist example per provider, channels, probes, dashboards, alerts);
    - `INSTALL.md` links to them;
    - `CI-CD.md` covers the Caddy recreate step and the channel files.
12. **Verify locally (colima):**
    - the full stack with the generic `ci.env`, `ALERT_CHANNEL=none`, and the relay from `:edge`;
    - switch to `ALERT_CHANNEL=telegram` with dummy values and confirm the contact point switches;
    - switch back to `none` and confirm it's removed;
    - `PROBE_TARGETS` both `[]` and a list.
13. **Before merge:**
    - Add the §8 keys to the VM `.env` with the backup+append pattern, never echoing values.
    - **Dry run on the VM** in a temp dir with the new files plus a copy of the prod `.env`:
      - `docker compose -f … config` → diff the Grafana environment and Caddy env against the running containers' env (`docker inspect`), and the probe targets against the current config;
      - `caddy adapt` → compare the site list.
14. **Ship:**
    1. Merge.
    2. CD recreates Caddy (confirm in the log) and syncs the observability stack.
    3. Run the Phase 1 §10.4 checklist: sites 200, the Grafana GitHub redirect, Basic auth 403, targets up, logs.
    4. Run the contact-point test, **with the user's OK**.

## PR-3 — Helm chart core (§5.1, §5.2, §5.5, §5.6)

1. **`charts/chakramcp/`:**
   - `Chart.yaml` (`apiVersion: v2`, `version: 0.0.0-dev`, `appVersion: edge`; the release sets both);
   - `values.yaml`, `values.schema.json`, `README.md` (the values table), `NOTES.txt`;
   - **Templates:**
     - `_helpers.tpl` (names, labels, `component: relay`);
     - `deployment.yaml`, `service.yaml`, `ingress.yaml`, `serviceaccount.yaml`;
     - `secret.yaml` (app secrets: existingSecret, or generated with a `lookup` keep);
     - `postgresql-{statefulset,service,secret}.yaml`, `redis-{deployment,service}.yaml`;
     - `tests/test-connection.yaml`.
   - With bundled Postgres, `DATABASE_URL` has no password and `PGPASSWORD` comes from the Secret via `secretKeyRef` (§5.2). The password never lands in a ConfigMap and needs no URL-encoding. External: `externalDatabase.url` or its `existingSecret`.
2. **`chart-ci.yml`:**
   - path-gated;
   - `helm dependency build` first (a no-op until PR-4), then lint, template (`ci/` and `ci-template/`), kubeconform (`-strict`, two K8s versions, CRD catalog);
   - `ct install` on kind with `ci/default-values.yaml` (`image.tag: edge`), then `helm test`;
   - a password case. `ct install --namespace` gets a fixed namespace holding a pre-created Secret whose password has URL-special characters (`@:/#%`). `ci/existing-secret-values.yaml` points `postgresql.auth.existingSecret` at it, and the release must become ready.
3. **`cli-release.yml`, job `chart`:**
   - `needs: [resolve-version, image-merge]`, `packages: write`;
   - `helm dependency build` (a no-op until PR-4), `helm package --version/--app-version <ver>` and `helm push oci://ghcr.io/delta-s-labs/charts`.
4. **Docs.** `docs/self-hosting/kubernetes.md` covers install, bundled vs external, Ingress/TLS, secrets and GitOps (the app and Postgres `existingSecret`s; PR-4 adds Grafana's), the percent-encoding note for an external `DATABASE_URL`, upgrades, and rollback across migrations.
5. **Verify.** kind locally (colima) with the default values, reaching the app through port-forward.
6. **Ship.** Merge, then ask the user whether to make chart-ci required.

## PR-4 — chart observability (§5.3, §5.4)

1. **`sync-chart-assets.sh`:** copies Overview (plus a variant without its Loki panel) + Logs, and filters `rules.yml` to the Kubernetes uid set (a small Python script inside it). It writes the `logs` group to its own `rules-logs.yaml`, and everything goes to `charts/chakramcp/files/`. CI runs it and fails on `git diff --exit-code`.
2. **Integration templates:**
   - a `servicemonitor.yaml` (Capabilities-gated, `labels` value, `honorLabels: true`, job relabel);
   - the dashboard ConfigMaps (Logs gated on `observability.loki.enabled`);
   - the alert-rules ConfigMap (`namespace` value; datasource UID substitution with `replace`; `rules-logs.yaml` only with `observability.loki.enabled`).
3. **Bundled:**
   - **Subchart dependencies**, conditions on `observability.bundled.enabled`:
     - prometheus-community `prometheus` (server only, remote-write receiver);
     - `loki` from `oci://ghcr.io/grafana-community/helm-charts` (single binary, caches/gateway/canary off);
     - `grafana` from `oci://ghcr.io/grafana-community/helm-charts` (datasources with fixed UIDs; sidecars for dashboards and alerts; auth and channel values);
     - `alloy` (a Deployment with 1 replica and a chart-provided config: `discovery.kubernetes` → scrape → remote_write; `loki.source.kubernetes` → process → Loki).
   - `helm repo add` prometheus-community and grafana (for Alloy), then `helm dependency update` → `Chart.lock`. chart-ci (`chart-repos` in `ct.yaml`, plus the static steps) and the release's `chart` job add the same repos before `helm dependency build`.
4. **CI.** `ci/bundled-values.yaml` installs on kind. Its checks run as `helm test` hook pods, since `ct` uninstalls afterwards: Grafana is healthy, the dashboards and rules exist (through the Grafana API), and `up{job="chakramcp"} == 1` (through the Prometheus API).
5. **Dependabot.** A `helm` entry for `/charts/chakramcp`. Confirm it raises PRs for the `oci://` dependencies; otherwise document manual bumps in `CI-CD.md`.
6. **Docs.** `observability.md` gains its Kubernetes section: integrate vs bundled, the kube-prometheus-stack `release` label and alerts sidecar, the rule set. `kubernetes.md` and `NOTES.txt` add Grafana's `admin.existingSecret` to the GitOps list.

## PR-5 — first versioned release (§8.5)

1. Ask the user for the version. Bump `backend/Cargo.toml` crate versions if desired, then push the tag `cli-v<ver>`. Afterwards, switch the Compose default image and docs from `:edge` to `:latest`.
2. **Verify:**
   - the release assets;
   - `ghcr.io/…:<ver>`, `:<X.Y>` and `:latest` on both architectures;
   - `helm pull oci://ghcr.io/delta-s-labs/charts/chakramcp --version <ver>`;
   - making the chart package public (a one-time toggle, with the user's OK);
   - a `helm install` from the OCI chart on kind.
