# Observability, Phase 1 — Implementation Plan

Derived from `2026-09-29-observability-phase1-design.md`, which was approved on 2026-09-29. Section numbers (§) refer to that spec.

The work ships as two PRs plus a few manual steps:
- **PR-A (backend + base compose)** is safe on its own: nothing scrapes it yet.
- **PR-B (the stack)** depends on PR-A being deployed.
- **The credits PRs** (#334, #335) merge after PR-B.

**Conventions**
- Commits end with the `Co-Authored-By` trailer; PR bodies end with the Claude Code footer.
- Ship pattern: branch → PR → CI green → squash-merge `--admin` → CD → verify *that commit's* CD run (`gh run list --commit <sha>`), because the concurrency group can drop an intermediate deploy.
- Keep `backend/.sqlx` fresh (`cargo sqlx prepare --workspace -- --tests`) if any query text changes.
- Clippy and fmt must be clean.
- Never print a secret. Read `.env` keys with `grep`/`cut`; never `source` the file.

---

## PR-A — backend telemetry + base compose

### A.0 Manual, before merging: swapfile on the VM

- Create a 2 GB `/swapfile` (mode 600), add it to `/etc/fstab`, and set `vm.swappiness=10` in `/etc/sysctl.d/99-swap.conf`.
- **Verify:** `swapon --show` lists it and `free -m` shows the swap. Nothing else on the VM changes.

### A.1 Dependencies (`backend/Cargo.toml` workspace + crate manifests)

- **`metrics = "0.24"`** — in `shared` and `relay`.
- **`metrics-exporter-prometheus = { version = "0.18", default-features = false }`** — in `shared`. We serve `/metrics` ourselves, so the built-in hyper listener and push gateway stay off.
- **`metrics-process = "2"`** — in `shared`.
- **`metrics-util = "0.20"`** — dev-dependency in `shared`, `relay` and `app`, for `DebuggingRecorder` in tests.
- **Verify:** `cargo tree -d | grep metrics` shows a single `metrics` version.

### A.2 `chakramcp_shared::telemetry` (new module; `tracing_init` folds into it)

1. **`LogFormat { Text, Json }`**
   - `LogFormat::parse(Option<&str>) -> (LogFormat, Option<String /*warning*/>)`: `json` or `text`, case-insensitive. Unknown or empty values give `Text` plus a warning, logged once init is done.
2. **`init_tracing(filter, format)`** (replaces `tracing_init::init`; the old fn stays as a thin wrapper until all callers move)
   - Text: the current formatter.
   - JSON: `fmt().json().flatten_event(true).with_current_span(true).with_span_list(false)`.
   - Also installs the **panic hook**: it logs `tracing::error!(panic.message, panic.location, "panic")`, then calls the previous hook.
3. **`install_metrics(addr: Option<SocketAddr>) -> Result<()>`**
   - `None` does nothing.
   - `Some(addr)`:
     - Build a `PrometheusBuilder` with `set_buckets_for_metric` for the two histograms, using the spec §4.3 buckets.
     - `install_recorder()`.
     - Spawn an upkeep task (`handle.run_upkeep()` every 5 s).
     - Bind and spawn a tiny axum router on `addr`: `GET /metrics` → `handle.render()`, anything else 404.
     - Register `describe_*!` help text for the catalogue.
     - Set `chakramcp_build_info{version=CARGO_PKG_VERSION, git_sha=option_env!("GIT_SHA").unwrap_or("unknown")}`. The `option_env!` sits in the **server binary crate** and is passed in, so only that crate rebuilds when the sha changes.
4. **`spawn_sampler(pools: Vec<(&'static str, PgPool)>)`**
   - Every 15 s: `metrics_process::Collector::collect()`, and per pool: `chakramcp_db_pool_connections{pool,state}` (in use = `size() - num_idle()`) and `chakramcp_db_pool_max_connections`.
   - It is only spawned when metrics are on.
5. **`normalize_method(&Method) -> &'static str`**: the seven standard methods, else `"other"`.
6. **`http_metrics` middleware** (`from_fn_with_state(service: &'static str, …)`)
   - Reads `MatchedPath`, else `"unmatched"`.
   - Times until the response returns and records `chakramcp_http_requests_total` and `chakramcp_http_request_duration_seconds`.
7. **`request_id` middleware**
   - Always mints `Uuid::now_v7()`, overwriting any inbound `x-request-id`, and stores it in a `RequestId` request extension.
   - Sets `x-request-id` on the response.
8. **`trace_layer()`**
   - `TraceLayer::new_for_http().make_span_with(...)` builds an **INFO** span `http_request{request_id, method, route}`.
   - Default on-request/on-response stay at DEBUG, and on-failure stays at ERROR.
9. **Metric-name constants** (`pub const HTTP_REQUESTS_TOTAL: &str = …`), so the relay records the exact catalogue names.

### A.3 Routers (`app/src/lib.rs`, `relay/src/lib.rs`)

- **Layer order.** The last `.layer()` is outermost; innermost first:
  1. routes
  2. (relay) `usage_middleware`
  3. `with_state`
  4. cors
  5. `trace_layer()`
  6. `http_metrics(service)`
  7. `request_id`

  `request_id` runs first on the way in, so the span sees the id. Metrics wrap everything except id minting.
- **Service labels:** `"app"` and `"relay"`.
- **Verify** with a router test (A.9) that a 404 on an unknown path is counted as `unmatched`, which confirms `Router::layer` also covers the fallback. If it doesn't, attach the layers to the fallback explicitly.

### A.4 Binaries and config

- **`server/src/main.rs`**
  - `ServerFile` gains `metrics_addr: Option<String>` and `log_format: Option<String>`; `ServerConfig` gains the parsed values.
  - Env wins: `METRICS_ADDR` and `LOG_FORMAT`, as with every other field. An invalid `METRICS_ADDR` is a startup error: an explicit, wrong bind address shouldn't pass silently.
  - `start()`: `init_tracing` → `install_metrics(cfg.metrics_addr)` → `spawn_sampler(vec![("main", pool.clone())])`.
  - `migrate()`: tracing only.
  - The `init` template gains commented `# metrics_addr = "127.0.0.1:9464"` and `# log_format = "json"` lines.
- **`app/src/main.rs`, `relay/src/main.rs`:** the same env-only wiring.
- **Credits:** at startup, set `chakramcp_credits_stale_after_seconds` from `credits.stale_after()`.

### A.5 Invocation outcomes

- Add `relay::telemetry` with `record_invocation_outcome(mode: Mode, status: &str, elapsed_ms: i64, count: u64)`.
  - It ignores `count == 0`.
  - The duration histogram is observed only for `succeeded | failed | timeout`.
  - Unknown statuses are ignored, with a `debug!`.
- **Classify every `relay_invocations` write.** Current list:

  | Site | What it is | Metric |
  |---|---|---|
  | `forwarder.rs` `persist_invocation` | push, terminal | `mode=push`; status/elapsed as persisted |
  | `invoke.rs:262` `record_terminal` | terminal insert (`rejected` at `:516`, `:834`) | `mode` from the caller's context: pull for queued/REST invocations, push if the call site is the A2A/push path. Decide per caller when reading it. |
  | `invoke.rs:677`, `invoke.rs:875` | enqueue inserts | check whether pending (no metric) or terminal |
  | `invoke.rs:969`, `mcp.rs:1004` | claim → `in_progress` | no metric |
  | `invoke.rs:1220` result endpoint | terminal update | `mode=pull`; count = `rows_affected()` |
  | `mcp.rs:1120` respond tool | terminal update | `mode=pull`; count = `rows_affected()` |
  | `grants.rs:482` revoke | bulk `pending → rejected` | `mode=pull`; count = `rows_affected()` |
  | `inbox_bridge.rs:136` | A2A inbox insert | check status; likely pending |
  | `invoke.rs:307`, `:645` | `trust_snapshot` updates | no metric |
  | `grants.rs:653`, `reviews.rs:848`, `invoke.rs:2752`, `a2a.rs:1421/1481`, `app/.../usage.rs:1001` | inside `#[cfg(test)]` | confirm, no metric |

- Record the metric only **after** the write succeeds. For transactions, record after `commit()`.
- Rows affected: `sqlx`'s `execute()` result `.rows_affected()`. The revoke UPDATE currently discards the result; capture it.

### A.6 MCP tool calls (`relay/src/handlers/mcp.rs:95`)

- In the `"tools/call"` branch:
  - Read the tool name from `req.params`.
  - Map it to itself if it's in the relay's tool registry (a `const` list next to `tools/list`), else to `"unknown"`.
  - Increment `chakramcp_mcp_tool_calls_total{tool, result}` with `ok` or `error` from the `Result`.
- **Verify:** the tool list matches `tools/list` output. Add a unit test that iterates the registry.

### A.7 Usage limits

- **`limits/mod.rs` `enforce`:** increment `chakramcp_limit_refusals_total{kind, enforced}` on the four refusal branches only: credits or rate × enforced or shadow.
- **`limits/rate.rs` `check_redis`:** increment `chakramcp_rate_limiter_errors_total` at both fail-open `Err(e)` arms.

### A.8 Credits worker (`relay/src/limits/credits/worker.rs`, `cache.rs`)

- **`account()` loop:**
  - `chakramcp_credits_accounting_runs_total{result}`: `ok`, `skipped` (advisory lock not acquired) or `error`.
  - `chakramcp_credits_charges_total` += rows drained.
  - `chakramcp_credits_queue_depth` from the existing `queue_depth` result.
- **`refresh_switches()`:**
  - `chakramcp_credits_switch_refreshes_total{result}`.
  - On success: set `chakramcp_credits_switches_last_refresh_timestamp_seconds` to the unix time now, and `chakramcp_credits_blocked_accounts` to `blocked_count()`.
  - Every tick: `chakramcp_credits_switches_stale` from `is_stale()`.
- **Worker pool:**
  - Set `chakramcp_db_pool_connections{pool="credits_worker",…}` inside the worker loop, since the worker owns its pool.
  - Or return the pool from `spawn_worker` so `spawn_sampler` can take it. Pick whichever keeps `spawn_worker`'s signature stable for #334.
- Keep the edits small and local; #334 rebases on top of them.

### A.9 Tests

- **Recorder in tests:**
  - Synchronous tests use `metrics::with_local_recorder(&DebuggingRecorder, || …)`.
  - `#[tokio::test]` and `#[sqlx::test]` hold `set_default_local_recorder` (current-thread runtime), then read a `Snapshotter`.
- **Where the tests go:**
  - **`shared::telemetry`:**
    - method normalisation;
    - `LogFormat::parse` (`json`, `text`, unknown → text + warning);
    - `/metrics` renders catalogue names;
    - `install_metrics(None)` binds nothing;
    - a request-id round trip (inbound id replaced, header present);
    - the panic hook logs at ERROR (capture with a test subscriber).
  - **app router (`tower::ServiceExt::oneshot`):**
    - `/healthz` is counted under its route template;
    - an unknown path is counted as `unmatched`;
    - a `PROPFIND` method is counted as `other`.
  - **relay:**
    - one test per instrumented invocation site (push success / failure / timeout through the existing forwarder tests; the result endpoint; MCP respond; revoke bulk count; `record_terminal` rejection);
    - a guarded UPDATE that matches nothing records nothing;
    - rejected rows don't feed the histogram;
    - `enforce`: refusal branches count, the allowed path doesn't;
    - worker gauges after one `account()` + `refresh_switches()` pass.
- **Verify:** `cargo test --workspace` (local Postgres + Redis, as in `backend-ci`), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.

### A.10 Log hygiene audit

- Grep every `tracing::` / `info!` / `warn!` / `error!` / `debug!` call in `backend/` for fields or format args that could carry secrets:
  - `token`, `secret`, `password`, `authorization`, `bearer`, `api_key`, `cookie`, `body`, `payload`, raw `headers`.
- Fix any hit: drop the field or log a fingerprint (length or prefix) instead.
- Note the result in the PR description.

### A.11 Compose + CD (`infra/docker-compose.prod.yml`, `.github/workflows/cd.yml`)

- **Logging anchor.** Add an `x-logging: &logging` anchor:
  ```yaml
  driver: journald
  options: { mode: non-blocking, max-buffer-size: 4m, labels: com.docker.compose.service }
  ```
  Put `logging: *logging` on `pg`, `redis`, `migrate`, `relay` and `caddy`.
- **Relay env:** `METRICS_ADDR: "0.0.0.0:9464"`, `LOG_FORMAT: json`.
- **Network:** declare `networks.observability: {driver: bridge}`. `caddy.networks` becomes `[internal, observability]`.
- **Header comment:** update it; mention the journald driver and the one-time Caddy recreate.
- **`cd.yml`:** the cargo build step exports `GIT_SHA: ${{ github.sha }}`, shortened in the step to 7 characters to match the image tag.
- **Verify:** `docker compose -f infra/docker-compose.prod.yml config -q`, with dummy env values.

### A.12 Local verification

- Run `chakramcp-server start` locally with `METRICS_ADDR=127.0.0.1:9464 LOG_FORMAT=json`:
  - `curl -s 127.0.0.1:9464/metrics | grep chakramcp_` shows the catalogue, including `build_info`, the pool gauges, `process_*`, and the credits gauges;
  - log lines are JSON and carry `request_id`;
  - responses carry `x-request-id`.
- **Performance** with macOS's built-in `ab`:
  - Run `ab -n 20000 -c 50 http://127.0.0.1:8090/healthz` and against one authenticated relay GET, metrics on vs off.
  - Compare p50/p99 over 3 runs each. Expect no difference beyond noise; if there is one, profile before shipping.

### A.13 Ship PR-A

1. Open the PR and get CI green: backend-ci, sqlx cache, security scan.
2. **Pick a quiet moment:** check the last hour's `usage_events` count on the VM (a read-only `psql`).
3. Merge. CD recreates `relay`, and also `pg` and `redis` because their definitions changed: a few seconds of blips.
4. Verify that commit's CD run.
5. **Manually, right after:** `cd /opt/chakramcp && docker compose up -d --no-deps caddy`. This recreates Caddy onto journald and the observability network, about a 2 s blip.
6. **Verify on the VM:**
   - `docker compose exec relay wget -qO- 127.0.0.1:9464/metrics | head` shows the catalogue;
   - `docker compose logs --tail 5 relay` is JSON;
   - `journalctl -o json -n 5 COM_DOCKER_COMPOSE_SERVICE=relay` shows entries with the service field;
   - `docker inspect` shows the journald driver on all four containers;
   - the site and sign-in still work;
   - `curl` from the internet to `:9464` fails, because the port isn't published.

---

## PR-B — the observability stack

### B.0 Manual, before merging (each is a no-op for the running system)

1. **DNS:** create an `A` record `grafana` → `54.84.88.246` in the Netlify DNS zone for `chakramcp.com`, through the Netlify API/CLI. Verify with `dig +short grafana.chakramcp.com`.
2. **VM `.env`:**
   - Generate `GRAFANA_ADMIN_PASSWORD` and `PG_MONITOR_PASSWORD` **on the VM** (`openssl rand -hex 24`) and append them with the backup-then-append pattern already used. Never print them.
   - Write `GRAFANA_ROLE_ATTRIBUTE_PATH`: the operator's numeric GitHub id as a JMESPath number literal, plus the public e-mail.
   - **Check the expression before relying on it:** evaluate it with Python's `jmespath` against `gh api user`. It must yield `GrafanaAdmin`, and a doctored copy with another id and e-mail must yield `''`.
3. **GitHub secrets:** set `TELEGRAM_BOT_TOKEN` and `TELEGRAM_CHAT_ID` with `gh secret set`, piping the values from the VM `.env` over SSH so they never appear in output.

### B.1 `infra/observability/compose.yml` (overlay: adds only)

- **Images:** pin the latest stable versions of `grafana/alloy`, `prom/prometheus`, `grafana/loki` and `grafana/grafana` (OSS), resolved at implementation time.
- **Every service:**
  - `restart: unless-stopped`;
  - a `mem_limit` (Prometheus 256m; Loki, Alloy and Grafana 192m each) and `GOMEMLIMIT` at about 80% of it;
  - the same journald `logging` block, repeated here because anchors don't cross files;
  - named volumes for data;
  - config mounted as **directories**, read-only.
- **Alloy:**
  - `user: "0"`, `cap_drop: [ALL]`, `security_opt: [no-new-privileges:true]`, `read_only: true`, `tmpfs: /tmp`;
  - mounts: `/var/log/journal`, `/run/log/journal` and `/etc/machine-id` (all read-only), `./observability/alloy` → `/etc/alloy` (ro), and the `alloy_data` volume;
  - networks: `internal` and `observability`;
  - env: `PG_MONITOR_PASSWORD`.
- **Prometheus:**
  - flags: `--config.file`, `--storage.tsdb.retention.time=15d`, `--storage.tsdb.retention.size=2GB`, `--web.enable-remote-write-receiver`, `--web.enable-lifecycle`;
  - network: `observability`.
- **Loki:** `-config.file`; network `observability`.
- **Grafana:**
  - network: `observability`;
  - env:
    - `GF_SERVER_ROOT_URL`, `GF_SECURITY_ADMIN_PASSWORD=${GRAFANA_ADMIN_PASSWORD}`, `GF_SECURITY_COOKIE_SECURE=true`;
    - `GF_AUTH_DISABLE_LOGIN_FORM=true`, `GF_AUTH_GITHUB_{ENABLED,CLIENT_ID,CLIENT_SECRET,SCOPES,ALLOW_SIGN_UP,AUTO_LOGIN,ROLE_ATTRIBUTE_STRICT,ALLOW_ASSIGN_GRAFANA_ADMIN}`;
    - `GF_AUTH_GITHUB_ROLE_ATTRIBUTE_PATH=${GRAFANA_ROLE_ATTRIBUTE_PATH:?set GRAFANA_ROLE_ATTRIBUTE_PATH in .env}`;
    - `GF_USERS_ALLOW_SIGN_UP=false`, `GF_AUTH_ANONYMOUS_ENABLED=false`, the analytics/update-check/news flags off;
    - `GF_UNIFIED_ALERTING_EXECUTE_ALERTS=${GF_UNIFIED_ALERTING_EXECUTE_ALERTS:-true}`;
    - `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`.
- No `depends_on` on base services.

### B.2 Prometheus — `prometheus/prometheus.yml`

Global settings only (`scrape_interval` is irrelevant; there are no scrape jobs). Alloy writes into it.

### B.3 Loki — `loki/loki.yml`

- Single binary, `auth_enabled: false`, filesystem storage, `tsdb` schema `v13`.
- Compactor with `retention_enabled: true` and `retention_period: 336h`.
- Modest `limits_config`; `server.log_level: warn`.

### B.4 Alloy — `alloy/config.alloy`

- **Destinations:** `prometheus.remote_write "local"` → `http://prometheus:9090/api/v1/write`; `loki.write "local"` → `http://loki:3100/loki/api/v1/push`.
- **Scrapes:**
  - `relay:9464` (job `chakramcp`, 15 s);
  - `caddy:2020` (job `caddy`, 15 s);
  - the exporters below (30 s);
  - blackbox (60 s);
  - self and stack (30 s), with a `prometheus.relabel` **keep-list**.
- **Exporters:**
  - **`unix`:** collector set trimmed; the filesystem collector limited to the data-volume mount.
  - **`postgres`:**
    - DSN `postgres://chakramcp_monitor:${PG_MONITOR_PASSWORD}@pg:5432/chakramcp?sslmode=disable`, read with `sys.env`;
    - `enabled_collectors` = the full default set plus `postmaster`. **Copy the default list from the pinned exporter version's source.**
  - **`redis`:** `redis:6379`.
  - **`blackbox`:** the `http_2xx` module; the four targets from spec §5.
- **Logs:**
  - `loki.source.journal` with `relabel_rules`:
    - keep entries that have `__journal_com_docker_compose_service`, or where `__journal__transport` is `kernel`;
    - set `service` from the compose field, or `kernel`;
  - `loki.process`:
    - `stage.json` → `level` (lower-cased), plus `stage.labels`;
    - drop below warn for `alloy|loki|prometheus|grafana`.
  - Check the actual journal field names on the VM (`journalctl -o json`) before writing the relabel rules.

### B.5 Grafana provisioning

- **`datasources/`:** Prometheus (uid `prometheus`) and Loki (uid `loki`).
- **`dashboards/`:** a file provider pointing at `/var/lib/grafana/dashboards` with `updateIntervalSeconds: 30`.
- **`alerting/`:**
  - **Contact point:** Telegram, with `bottoken: $TELEGRAM_BOT_TOKEN` and `chatid: $TELEGRAM_CHAT_ID` (env interpolation), and a compact message template.
  - **Policy:** group by `alertname`, 30 s / 5 m / 4 h.
  - **Rules:** the spec §8 table, one rule group per area, "no data" set where the spec says:
    - Credit stale: `timestamp(m) - m > chakramcp_credits_stale_after_seconds`.
    - Crash loop: deploy subtraction for the relay; `resets(redis_uptime_in_seconds)`; `changes(pg_postmaster_start_time_seconds)`.
    - OOM: `increase(node_vmstat_oom_kill[10m]) > 0`.
    - Error-log spike: a Loki query.

### B.6 Dashboards — `grafana/dashboards/{overview,infrastructure,logs}.json`

- Build the three dashboards to the spec §7 panel lists, with datasource uids `prometheus`/`loki`.
- Health routes are filtered out of the traffic panels.
- Restart annotations come from `changes(process_start_time_seconds{job="chakramcp"}[1m]) > 0`, with `git_sha` from `chakramcp_build_info`.
- Author them in a local Grafana (B.12), export, and strip ids and versions.

### B.7 `infra/Caddyfile`

- **Global options:** `metrics { per_host }`.
- **`:2020`:** `metrics`.
- **`grafana.chakramcp.com`:**
  ```
  encode zstd gzip
  @basic header_regexp Authorization (?i)^basic\s
  respond @basic 403
  reverse_proxy grafana:3000
  handle_errors { respond "{err.status_code} {err.status_text}" }
  ```
- **Verify:** `caddy validate` in a container against the new file.

### B.8 `cd.yml`

- **Filters:** `backend` becomes `backend/**` + `infra/*`. There is no observability filter.
- **`which`:** add the `observability` option.
- **New `deploy-observability` job**, per spec §9.2:
  - `needs: [detect, <backend deploy job>]`;
  - `if: always() && needs.detect.result == 'success' && (needs.<backend>.result == 'success' || needs.<backend>.result == 'skipped')`.
- **Steps:**
  1. **SSH setup** (reuse the backend job's).
  2. **Hash, sync, hash:** a `sha256` of each config dir (`alloy/`, `prometheus/`, `loki/`, `grafana/provisioning/`, `grafana/dashboards/`) on the VM, before and after. Sync with `rsync -a --delete infra/observability/ → /opt/chakramcp/observability/`.
  3. **Monitoring role:** existence check first; create it only if missing (spec §9.2).
  4. **Start:** `docker compose -f docker-compose.yml -f observability/compose.yml up -d alloy prometheus loki grafana`.
  5. **Reload changed configs:**
     - **Prometheus:** `docker compose exec -T prometheus wget -qO- --post-data= http://127.0.0.1:9090/-/reload`.
     - **Alloy:** `POST http://alloy:12345/-/reload`, sent from the Prometheus container (it has busybox `wget`).
     - **Grafana alerting and datasources:** `POST /api/admin/provisioning/{alerting,datasources}/reload`, run in the Grafana container with the admin password read from `.env` (`grep`/`cut`) and passed through the environment.
     - **Loki:** restart.
     - Fail on any non-2xx. Then assert `prometheus_config_last_reload_successful == 1` and `alloy_config_last_load_successful == 1`.
  6. **Status:** wait for health and print `docker compose ps` for the four services.

### B.9 `.github/workflows/observability-ci.yml`

- **Trigger:** `pull_request` and `push` to main. A job-internal paths check covers `infra/**` and the workflow file.
- **Static checks:**
  - `promtool check config` (the pinned `prom/prometheus` image);
  - `loki -verify-config`;
  - `alloy fmt`;
  - `caddy validate`;
  - `docker compose … --env-file infra/observability/ci.env config -q`;
  - a small Python check: every dashboard JSON parses and uses only uids `prometheus`/`loki`, and the alerting YAML parses.
- **Smoke test** (ubuntu runner):
  1. `up -d pg redis alloy prometheus loki grafana` with `ci.env`.
  2. Create the monitoring role with the CD script, factored into `infra/observability/scripts/ensure-monitor-role.sh` so CI and CD share it.
  3. Poll Prometheus for about 60 s, then assert:
     - `pg_up == 1` and `redis_up == 1`;
     - one metric per Postgres panel is present;
     - `node_load1` is present;
     - the stack targets are up.
  4. Loki `query_range` for `{service="pg"}` returns lines.
  5. Grafana API (admin basic auth from inside the network): 3 dashboards, the rule groups, and the Telegram contact point exist.
  6. Tear down.

### B.10 `.github/workflows/uptime.yml`

- **Schedule:** `*/10 * * * *`, plus `workflow_dispatch` with a `test_message` boolean.
- **Check:** curl with retries and timeouts against `https://relay.chakramcp.com/healthz`, `https://app.chakramcp.com/healthz` and `https://grafana.chakramcp.com/api/health`.
- **Alert:** on any failure, or when `test_message` is set, post to Telegram through the secrets. Mark test messages clearly as tests.
- **Housekeeping:** `concurrency: uptime`, and minimal `permissions: {}`.

### B.11 `.github/dependabot.yml`

- Add a `docker-compose` ecosystem entry for `/infra/observability`, weekly, grouped.
- The existing auto-merge handles patch bumps.

### B.12 Local and dev files

- **`infra/observability/compose.dev.yml`:**
  - json-file logging on every service;
  - Alloy's journal and machine-id mounts reset with `!reset`;
  - Grafana published on `127.0.0.1:3000`, login form on, GitHub off, and a throwaway admin password;
  - the relay image from a local build.
- **`infra/observability/ci.env`:** dummy values for every interpolated variable, plus `GF_UNIFIED_ALERTING_EXECUTE_ALERTS=false`.
- **Local run (Mac):** base + overlay + dev override, starting everything except Caddy. Use it to author the dashboards and to check that every Alloy component except the journal is healthy.

### B.13 Docs

**`docs/CI-CD.md`** gains an observability runbook:
- the two-file compose alias;
- where each secret lives, and rotating the monitoring password and the Grafana admin;
- break-glass admin login from inside the VM;
- an SSH tunnel to Prometheus and Loki;
- what CD does on merge;
- what to do when the uptime workflow is disabled after 60 idle days.

### B.14 Ship PR-B

1. Confirm B.0 is complete (DNS resolves; the `.env` keys exist, checked by name only; the GitHub secrets exist).
2. Open the PR and get CI green, including the smoke test.
3. Merge. Then verify:
   - The backend job rebuilds, recreates the relay, and reloads Caddy.
   - `deploy-observability` brings the stack up.
   - Verify *that commit's* CD run.
4. **Production check, per spec §10.4:**
   - all targets up;
   - logs from every service in Loki;
   - every panel has data;
   - Grafana contact-point test → Telegram, with your OK for the test message;
   - your GitHub sign-in works;
   - a non-allowlisted account is refused (you test with a second GitHub account, or I test with a doctored expression in a throwaway local Grafana);
   - Basic auth from outside → 403;
   - `:9464` and `:2020` are unreachable from outside;
   - the uptime workflow's dispatch with `test_message` arrives.
5. Make `observability-ci` a required check on `main`, with your OK, since it's a repo setting.

---

## After PR-B

1. **#334:** rebase onto main. Resolve the worker conflicts; A.8's edits are small and local. Re-run tests and merge.
2. **#335:** rebase and merge.
3. Confirm the credits panels are populated.
4. **Memory notes:**
   - the observability layout;
   - journald logging;
   - the two-file compose form on the VM;
   - never `source` `.env`.
