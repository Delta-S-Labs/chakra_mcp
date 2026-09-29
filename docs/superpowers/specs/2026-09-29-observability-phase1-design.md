# Observability, Phase 1 — metrics, logs, dashboards and alerts for production

**Status:** design, awaiting review
**Date:** 2026-09-29
**Scope:** the current production deployment (one Lightsail VM running Docker Compose).
**Phase 2 (separate spec, later):** packaging the same stack for open-source self-hosters on Docker Compose and Kubernetes.

## 1. Goal

See how production is doing, and hear about it when it isn't, from one private Grafana:

- metrics from the backend (traffic, errors, latency, invocations, usage limits, credits worker, DB pool, process), the host, Postgres, Redis and Caddy;
- the logs of every container, searchable;
- a small set of alerts delivered to Telegram;
- an outside check that still alerts when the whole VM is down.

The credits PRs (#334 backend, #335 UI) merge after this lands, so they ship with their metrics visible.

**Out of scope for Phase 1:** distributed tracing (OpenTelemetry/Tempo), frontend (Netlify) logs, per-account analytics (the app already has usage pages), Alertmanager, HA or remote storage, long-term retention, per-container cgroup metrics (cAdvisor; see §5), self-hoster packaging.

## 2. Constraints

| Constraint | Consequence |
|---|---|
| The VM has 2 vCPU, 1.9 GB RAM, **1.26 GB available, no swap**, 49 GB free disk. | Lean stack with hard memory caps, a swapfile as an OOM safety net, alerts on memory and disk. |
| **Never hamper invocation performance** (standing project rule: the invocation path only reads in-memory state; accounting runs in background workers). | Instrumentation on the request path is in-memory atomics only: no I/O, no locks held across `.await`, no new DB queries. Anything that needs the DB is sampled by a background task. Container logging must never block the app's stdout. |
| The repo is **public**. | No secret or personal data (e-mail addresses, allowlists) in any committed file; they come from the VM's `.env` or GitHub secrets. |
| Monitoring shares the box it watches. | An external uptime check from GitHub Actions covers the "VM is down" case. |
| CD deploys automatically on merge (`cd.yml`). | Every rollout step must be something CD's run on merge does safely, or a manual step done *before* the merge that is a no-op for the running system. |

## 3. Architecture

```
                   Internet
                      │ 80/443 (the only published ports, unchanged)
                  ┌───▼───┐
                  │ Caddy │── grafana.chakramcp.com ──► Grafana :3000
                  └─┬───┬─┘
       relay / app ◄┘   └── (on both networks; :2020 /metrics, internal only)
 ┌──────────────── internal network (existing) ─────────────────────┐
 │ relay  :8080 app  :8090 relay  :9464 /metrics (new, internal)    │
 │ pg :5432     redis :6379                              Alloy ─────┼──┐
 └──────────────────────────────────────────────────────────────────┘  │
 ┌──────────────── observability network (new) ─────────────────────┐  │
 │ Caddy · Grafana · Prometheus (15 d, ≤ 2 GB) · Loki (14 d) · Alloy ◄┼──┘
 └──────────────────────────────────────────────────────────────────┘
   Alloy ─ remote_write ─► Prometheus        Grafana ─► Prometheus, Loki
   Alloy ─ push ─────────► Loki              Grafana ─► Telegram, GitHub (egress)
   Alloy ◄─ host journal (read-only mount): every container's logs
```

| Component | Role | Networks | Memory cap |
|---|---|---|---|
| **Alloy** | The only collector. It scrapes every metrics source, runs the built-in exporters (host, Postgres, Redis, HTTPS probes), reads container logs from the host journal, and forwards metrics to Prometheus and logs to Loki. | internal, observability | 192 MB |
| **Prometheus** | Metrics store and query engine. Receives remote-write only; scrapes nothing itself. | observability | 256 MB |
| **Loki** | Log store (single binary, filesystem). | observability | 192 MB |
| **Grafana** | Dashboards and alerting. The only component reachable from outside, through Caddy. | observability | 192 MB |

- Each service also gets `GOMEMLIMIT` at about 80% of its cap, so it garbage-collects before the kernel kills it. Caps total about 830 MB; expected steady use is about 420 MB.
- Images are pinned to exact versions, resolved when the stack is built.
- **The `observability` network and Caddy's membership of it are declared in the base compose file** (`infra/docker-compose.prod.yml`, PR-A). The overlay therefore never changes a base service's definition, and CD's relay deploys and the observability deploys never recreate each other's containers.
  - Grafana is on `observability` only; Caddy bridges `internal` and `observability`.
  - Alloy is on both networks because it scrapes the relay, Postgres, Redis and Caddy.
- `observability` is an ordinary bridge network, because Grafana needs egress to GitHub (OAuth) and Telegram and Alloy needs egress to the probe targets.
- No new host ports are published.

**Alloy's privileges are deliberately minimal:**
- it runs with no Docker socket or Docker API access, no host `/` or host `/proc` mounts, and not privileged;
- `cap_drop: [ALL]`, `no-new-privileges`, and a read-only root filesystem;
- its only host mounts are the journal directories and `/etc/machine-id`, read-only.

It runs as uid 0 with every capability dropped. The journal files are `root:systemd-journal 0640`, so owning them is enough to read them, and nothing else from the host is mounted.

Reading container configuration (`docker inspect`) would expose every service's secrets through their environment variables. That is why logs come from the journal rather than the Docker API (§4.4, §5).

## 4. Backend instrumentation

### 4.1 Configuration

| Setting (env / `server.toml`) | Default | Prod |
|---|---|---|
| `METRICS_ADDR` / `metrics_addr` | unset: **no metrics listener** | `0.0.0.0:9464` |
| `LOG_FORMAT` / `log_format` | `text` | `json` |

- Metrics are opt-in, so a self-hoster never exposes `/metrics` by accident.
- An unknown `LOG_FORMAT` falls back to `text` and logs a warning; a typo in a log setting never stops the server.
- The shared helpers live in `chakramcp-shared`. `chakramcp-server` (the prod binary) wires them in, and the standalone `app`/`relay` binaries use the same helpers.

### 4.2 The `/metrics` listener

- Crates: `metrics` (the facade), `metrics-exporter-prometheus` (the recorder and text rendering), `metrics-process` (standard `process_*` metrics).
- Serves `GET /metrics` on `METRICS_ADDR`: a separate tiny axum router with nothing else on it.
- A background task runs the exporter's upkeep, which drains the histogram buffers, every 5 s.
- Caddy never routes to this port and compose does not publish it.
- When `METRICS_ADDR` is unset, no recorder is installed. The `metrics` macros then become no-ops, which is also what tests see unless they install a local recorder.

### 4.3 Metric catalogue

All metric names are prefixed `chakramcp_` except the standard `process_*` set. **Label values come from bounded sets only:** route templates, status codes, fixed enums, normalised methods. Never account or agent IDs, raw paths, or other client input.

| Metric | Type | Labels | Recorded where |
|---|---|---|---|
| `chakramcp_http_requests_total` | counter | `service` (`app`\|`relay`), `method`, `route`, `status` | a middleware on both routers |
| `chakramcp_http_request_duration_seconds` | histogram | `service`, `method`, `route` | same middleware (time until response headers are ready) |
| `chakramcp_invocations_total` | counter | `mode` (`push`\|`pull`), `status` (`succeeded`\|`failed`\|`timeout`\|`rejected`) | every write that moves invocations into a terminal status |
| `chakramcp_invocation_duration_seconds` | histogram | `mode` | same, but **only for `succeeded`, `failed`, `timeout`**. `rejected` rows carry `elapsed_ms = 0` and would distort percentiles. Push: round trip to the agent. Pull: enqueue → result, the same `elapsed_ms` the audit row stores. |
| `chakramcp_mcp_tool_calls_total` | counter | `tool`, `result` (`ok`\|`error`) | the `tools/call` dispatcher in `relay/src/handlers/mcp.rs`; unknown tool names are recorded as `unknown` |
| `chakramcp_limit_refusals_total` | counter | `kind` (`credits`\|`rate`), `enforced` (`true`\|`false`) | `limits::enforce`, on its refusal branches only (the allowed path is untouched) |
| `chakramcp_rate_limiter_errors_total` | counter | — | `limits::rate`, where a Redis error makes the limiter fail open (both sites in `check_redis`) |
| `chakramcp_credits_queue_depth` | gauge | — | credits worker, each sweep (the value it already computes) |
| `chakramcp_credits_accounting_runs_total` | counter | `result` (`ok`\|`skipped`\|`error`); `skipped` = the advisory lock is held elsewhere | credits worker |
| `chakramcp_credits_charges_total` | counter | — | credits worker (rows drained) |
| `chakramcp_credits_switch_refreshes_total` | counter | `result` (`ok`\|`error`) | credits worker |
| `chakramcp_credits_switches_last_refresh_timestamp_seconds` | gauge | — | credits worker, on a successful refresh |
| `chakramcp_credits_switches_stale` | gauge (0/1) | — | credits worker (`CreditCache::is_stale`) |
| `chakramcp_credits_stale_after_seconds` | gauge | — | at startup, from `CreditsConfig` (lets the alert compute staleness from outside the process) |
| `chakramcp_credits_blocked_accounts` | gauge | — | credits worker, after each refresh |
| `chakramcp_db_pool_connections` | gauge | `pool` (`main`\|`credits_worker`), `state` (`idle`\|`in_use`) | a sampler task, every 15 s |
| `chakramcp_db_pool_max_connections` | gauge | `pool` | same sampler |
| `process_*` (`cpu_seconds_total`, `resident_memory_bytes`, `open_fds`, `threads`, `start_time_seconds`, …) | mixed | — | `metrics-process`, collected by the same 15 s sampler |
| `chakramcp_build_info` | gauge (=1) | `version`, `git_sha` | at startup; CD passes `GIT_SHA` to the cargo build |

**HTTP labels**
- `route` is axum's `MatchedPath` template (e.g. `/v1/invocations/{id}`); requests that match no route are recorded as `unmatched`.
- `method` is normalised to `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE` or `OPTIONS`, and anything else becomes `other`. Hyper accepts arbitrary method tokens, so this keeps scanners from creating series.
- Health checks are recorded like any route; dashboards filter them out.

**Invocation terminal writes**
- These are counted through one helper, e.g. `record_invocation_outcome(mode, status, elapsed, count)`.
- The plan must enumerate every site that inserts an invocation with, or updates it to, a terminal status, found by grepping `relay_invocations`. Known sites include:
  - `forwarder::forward_push`
  - `POST /v1/invocations/{id}/result`
  - the MCP respond tool
  - `record_terminal` (policy rejections)
  - the bulk `UPDATE … WHERE status = 'pending'` on grant revoke in `handlers/grants.rs`
- Counts come from `rows_affected()`. A guarded update (`WHERE status = 'in_progress'`) that changed nothing records nothing, so retries are not double-counted; a bulk update records its row count.

**Histogram buckets**
- HTTP: 5, 10, 25, 50, 100, 250, 500 ms, 1, 2.5, 5, 10, 30 s.
- Invocations: 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 300, 1800 s, because pull-mode and human-in-the-loop invocations can take minutes.

**Performance rules**
- The cost on the request path is two `Instant::now()` calls and a few atomic increments behind a registry lookup, well under 1 µs per request.
- No DB query, Redis call, channel send, or lock held across `.await` is added to any request or invocation path.
- Gauges that need the DB, a pool, or `/proc` are set from background tasks that already exist (the credits worker) or from the new 15 s sampler.

**Expected size:** roughly 2–4 k series from the backend, dominated by the HTTP histogram (routes × methods × buckets).

### 4.4 Logs

- **JSON output.** `LOG_FORMAT=json` switches `tracing_subscriber` to JSON (the `json` feature is already enabled in the workspace). Each line carries timestamp, level, target, message, fields, and the fields of the current span.
- **Request span.** Both routers' `TraceLayer` get an INFO-level span per request with `request_id`, `method` (normalised) and `route`, so every log line a request produces carries its ID. Per-request start/finish events stay at their current DEBUG level: no new log volume at `info`. 5xx responses are already logged at ERROR by `TraceLayer`.
- **Request ID.** The server always mints its own ID; any client-supplied `x-request-id` is replaced, so logs can't be polluted with arbitrary values. It is returned as `x-request-id` on every response, so a user can quote it in a bug report.
- **Log driver.** Every service in the prod compose file logs through Docker's **`journald` driver**, via one `x-logging` anchor with these options:
  - `mode: non-blocking` and `max-buffer-size: 4m`: a slow log pipeline drops lines instead of ever blocking a container's stdout, which protects the invocation path;
  - `labels: com.docker.compose.service`, so each journal entry carries its compose service name.

  `docker compose logs` keeps working. Rotation and retention become journald's job; its defaults cap the journal at 10% of the disk, at most 4 GB. No container rotates its logs today (json-file, no options, no `daemon.json`), so this also fixes unbounded log growth.

  YAML anchors don't cross files, so `observability/compose.yml` repeats the same logging block for its four services.
- **Panics.** The backend installs no panic hook today, so a panic prints plain text with no `level` and would miss the error-log alert and the errors panel. A panic hook now logs the panic message and location through `tracing` at ERROR, then defers to the default hook.
- **Log hygiene.** The PR that turns on JSON logs includes an audit of existing log statements. No secrets, tokens, `Authorization` headers, request or response bodies, or passwords may be logged. Account and agent UUIDs are fine.
- **Local dev.** Plain text stays the default. Docker Desktop has no journald, so the local override (§10.3) switches the driver back to `json-file`.

## 5. Collection and storage

Alloy's configuration (`infra/observability/alloy/config.alloy`) has these pipelines:

| Source | How | Interval |
|---|---|---|
| Backend | scrape `relay:9464/metrics` | 15 s |
| Caddy | Caddy's `metrics` global option (per-host; unknown hosts collapse into `_other`), served by an internal-only `:2020` site; scrape `caddy:2020/metrics` | 15 s |
| Host | built-in `prometheus.exporter.unix`, reading the container's own `/proc` | 30 s |
| Postgres | built-in `prometheus.exporter.postgres` as `chakramcp_monitor`, a new login role that is a member of `pg_monitor` (read-only statistics). In Alloy's fork of the exporter, `enabled_collectors` is an **exclusive** list, so it names the full default set **plus** `postmaster` (for `pg_postmaster_start_time_seconds`). Naming only `postmaster` would silently switch every other Postgres metric off. | 30 s |
| Redis | built-in `prometheus.exporter.redis` | 30 s |
| HTTPS probes | built-in `prometheus.exporter.blackbox`, `http_2xx` against `https://relay.chakramcp.com/healthz`, `https://app.chakramcp.com/healthz`, `https://chakramcp.com/` and `https://grafana.chakramcp.com/api/health`; this also gives certificate expiry | 60 s |
| The stack itself | Alloy's own metrics and the Prometheus, Loki and Grafana `/metrics` endpoints, through a **keep-list** (process memory/CPU, `up`, and a few health series such as Prometheus head series and Loki ingestion errors) | 30 s |
| Logs | `loki.source.journal` → `loki.process` → Loki. It keeps container entries (those with the compose-service field) and kernel messages (`_TRANSPORT=kernel`, so OOM-killer reports are searchable), and drops the rest. The component's `matches` argument can only AND conditions, so this OR is done with relabel/drop rules. | streaming |

**Host metrics without host mounts**
- CPU, memory, load, pressure, `/proc/vmstat` (including `oom_kill`) and disk I/O are not namespaced, so the container's own `/proc` reports the host's values.
- Root-disk usage comes from a filesystem the container sees that lives on the host root disk: its own data volume.
- Host network counters are not available; Caddy's metrics cover traffic.
- The implementation verifies each of these on the VM. If one is wrong, the fallback is a read-only host `/proc` or `/sys` mount, **never** host `/`.

**Per-service health without cAdvisor.** cAdvisor was considered and dropped. It needs host `/sys`, `/var/lib/docker`, `/dev/kmsg` and usually privileged mode, and that access would reach `.env` and the Postgres volume. Instead:
- **Memory and CPU:** every component's own `process_*` metrics (the relay through `metrics-process`, the Go services natively, `redis_memory_used_*` for Redis). Postgres exposes neither, so its panel shows connections and activity, and host memory stands in for its footprint.
- **Restarts:**
  - changes in `process_start_time_seconds` (relay, Caddy, stack);
  - changes in `pg_postmaster_start_time_seconds`;
  - `resets(redis_uptime_in_seconds)`. Redis's own start-time metric is derived from uptime and jitters by a second, so it would look like a restart.
- **OOM kills:** `node_vmstat_oom_kill`, host-wide, which includes memory-cap kills; the kernel's journal message names the victim.

**Log labels**
- Only `service` (from the compose-service journal field) and `level`.
- `level` is extracted where a container logs JSON (relay, Caddy) and normalised to lower case.
- Everything else is parsed at query time with LogQL's `| json`.
- The stack's own services (Alloy, Loki, Prometheus, Grafana) are shipped at `warn` and above only, so Loki can't feed on its own logs.
- Caddy access logs stay off: the backend's HTTP metrics cover traffic, and access logs would add client IP addresses to storage.

**Retention**
- Prometheus: `--storage.tsdb.retention.time=15d` and `--storage.tsdb.retention.size=2GB`.
- Loki: 14 days through the compactor.
- Alloy keeps its write-ahead log and journal cursor on a volume, so restarts lose nothing.

## 6. Access and security

**DNS.** An `A` record `grafana.chakramcp.com → 54.84.88.246` in Netlify DNS, which hosts the `chakramcp.com` zone (NS1 name servers). It must exist **before PR-B merges**, because CD reloads Caddy with the new site on merge (§9.4).

**Caddy** gains:
- a `grafana.chakramcp.com` site that proxies to `grafana:3000`, and answers 403 to any request whose `Authorization` header starts with `Basic`, matched case-insensitively (`header_regexp`, `(?i)^basic\s`), so password auth only works from inside the VM;
- the `metrics` global option (per-host) and the internal `:2020` metrics site.

**Grafana sign-in**
- GitHub OAuth is the only way in: `disable_login_form = true` and `auto_login = true`, so the login page goes straight to GitHub.
- Uses the `chakramcp-grafana` OAuth app, whose callback is `https://grafana.chakramcp.com/login/github`. Its credentials are in the VM `.env` and were verified against GitHub.
- **The allowlist is not in the repo.** `role_attribute_path` comes from the VM `.env` (`GRAFANA_ROLE_ATTRIBUTE_PATH`). Compose refuses to start Grafana without it (`${VAR:?}`), and with `role_attribute_strict = true` an empty or non-matching role refuses the login.
  - Our value matches the operator's **numeric GitHub user id**, which is immutable, unlike a login that can be renamed and re-registered, **or** their e-mail. The e-mail only matches while it is public on the GitHub profile, which GitHub only allows for verified addresses. The result is `GrafanaAdmin`, with `allow_assign_grafana_admin = true`.
  - Every other account maps to an empty role and is refused.
  - This also makes the allowlist a plain setting for Phase 2 self-hosters.
  - The numeric id must be written as a JMESPath number literal (`` id == `123` ``). A quoted `'123'` never equals a number, and strict mode would then lock the operator out. The implementation checks the expression against GitHub's real `/user` response before relying on it.
  - The value contains parentheses, quotes and backticks, so scripts must never `source` the `.env` file. CD and the runbook read single keys with `grep`/`cut`.
- `allow_sign_up = true` (strict mode is the gate); the `[users] allow_sign_up` form sign-up is off.
- `cookie_secure = true`, `root_url = https://grafana.chakramcp.com`.
- Anonymous access, analytics reporting, update checks and the news feed are off.
- The built-in `admin` gets a random password (`GRAFANA_ADMIN_PASSWORD`, generated on the VM and never printed). It is break-glass only, usable from inside the VM because of the Caddy rule above.

**Exposure**

| Endpoint | Reachable from |
|---|---|
| Grafana UI | internet, through Caddy + TLS + GitHub allowlist |
| `relay:9464/metrics`, `caddy:2020/metrics` | Docker networks on the VM only |
| Prometheus, Loki, Alloy UIs / APIs | the observability network only (use an SSH tunnel to debug) |
| Docker API | nothing new: no container gets the socket |

**Secrets and settings, in `/opt/chakramcp/.env` (mode 600), never in the repo:**
- `TELEGRAM_BOT_TOKEN` ✅
- `TELEGRAM_CHAT_ID` ✅
- `GRAFANA_GITHUB_CLIENT_ID` ✅
- `GRAFANA_GITHUB_CLIENT_SECRET` ✅
- `GRAFANA_ADMIN_PASSWORD` (generate)
- `PG_MONITOR_PASSWORD` (generate)
- `GRAFANA_ROLE_ATTRIBUTE_PATH` (write)

The uptime check (§8) uses the GitHub Actions secrets `TELEGRAM_BOT_TOKEN` and `TELEGRAM_CHAT_ID`, copied from the VM without printing them.

**Keeping images patched.** Dependabot gets a `docker-compose` entry for `infra/observability/compose.yml`. Under the repo's existing auto-merge policy, patch bumps merge and deploy automatically; that is intended, because it gets Grafana security fixes out fast. Minor and major bumps wait for review.

## 7. Dashboards

Dashboards are provisioned from JSON in the repo (`infra/observability/grafana/dashboards/`). Datasources are provisioned with fixed UIDs (`prometheus`, `loki`), so the dashboard JSON is portable to Phase 2 unchanged.

1. **ChakraMCP — Overview**
   - request rate, 5xx ratio, p50/p95/p99 latency per service, and top routes;
   - invocations by mode/status and invocation latency;
   - MCP tool calls;
   - limit refusals and rate-limiter errors;
   - credits: queue depth, seconds since the last switch refresh against the stale window, blocked accounts;
   - DB pool in use vs max;
   - relay memory and CPU;
   - deploy/restart markers (annotations when `process_start_time_seconds` changes, labelled with `chakramcp_build_info`'s `git_sha`);
   - a recent-errors log panel.
2. **Infrastructure**
   - host CPU, memory, swap, disk, pressure, OOM kills;
   - per-service memory, CPU and restarts, from each component's own metrics;
   - Postgres: connections, transactions/s, cache hit ratio, locks waiting, deadlocks, DB size;
   - Redis: memory, ops/s, clients;
   - Caddy per-host requests and upstream errors;
   - probe status, latency, certificate expiry;
   - Prometheus series count and the stack's own memory.
3. **Logs**: a service and level picker, free-text search, log volume by level.

## 8. Alerting

Alerts use Grafana-managed rules, provisioned as files (`infra/observability/grafana/provisioning/alerting/`), with no Alertmanager.

**Contact point:** Telegram via `@chakramcp_bot`, to the chat in `TELEGRAM_CHAT_ID`. Provisioning files read both values from the environment.

**Notification policy**
- Group by alert name.
- `group_wait` 30 s, `group_interval` 5 m, `repeat_interval` 4 h.
- Resolved notifications on.

**Rules.** Thresholds are initial values, to be tuned after a week of data. "No data" is treated as firing where noted.

| Alert | Condition | For |
|---|---|---|
| Backend down | relay or app probe failing, **or** the backend scrape target down; no data = firing | 2 m |
| High 5xx ratio | 5xx / all > 5 % over 5 m, while traffic > 0.1 req/s | 5 m |
| Credit switches stale | `timestamp(m) - m > chakramcp_credits_stale_after_seconds`, where `m` = `chakramcp_credits_switches_last_refresh_timestamp_seconds`, or `chakramcp_credits_switches_stale == 1`. This measures the refresh's age at scrape time, so scrape and remote-write delay don't count; the prod stale window is 15 s (3 × the 5 s sweep). It is computed outside the flag, so it fires even if the refresh loop dies before updating it; no data = firing. | 2 m |
| Credit queue backlog | `chakramcp_credits_queue_depth > 5000` | 15 m |
| Postgres down | `pg_up == 0` | 1 m |
| Disk filling | root disk > 85 % used | 10 m |
| Memory low | `MemAvailable / MemTotal < 10 %` | 10 m |
| OOM kill | `increase(node_vmstat_oom_kill[10m]) > 0` | 0 m |
| Crash loop | a service restarted more than twice in 15 m (restart signals as in §5). Deploys don't count. Stack config changes are applied by reload, not restart (§9.2). For the relay the rule subtracts deploys: restarts − (distinct `git_sha` values in `chakramcp_build_info` over the window − 1) > 2. A crash loop right after a deploy still fires. | 0 m |
| Certificate expiring | any probed certificate expires in < 14 d | 1 h |
| Error-log spike | > 20 `level="error"` lines from the relay in 5 m | 5 m |
| Monitoring blind | Alloy's own metrics absent (the collector is down); no data = firing | 5 m |

**Outside check.** `.github/workflows/uptime.yml` runs every 10 minutes:
- curls the relay and app `/healthz` and Grafana's `/api/health`, with retries;
- on failure, sends a Telegram message through the same bot, using the GitHub secrets;
- repeats every 10 minutes while the outage lasts, which is acceptable;
- sends no recovery message.

It is the only alert path that survives the VM dying. Two limits apply:
- GitHub's cron can be late by several minutes.
- GitHub disables scheduled workflows in public repos after 60 days with no repository activity, and e-mails the owner when it does.

A `workflow_dispatch` input sends a clearly marked test message, to prove the path end to end.

## 9. Deployment

### 9.1 Files

```
infra/
  docker-compose.prod.yml   (PR-A) x-logging (journald) on every service; relay gets METRICS_ADDR, LOG_FORMAT;
                            declares the observability network; caddy joins it
  Caddyfile                 (PR-B) metrics global option, :2020 metrics site, grafana site
  observability/            (PR-B)
    compose.yml             the stack: overlay that only ADDS services and volumes
    compose.dev.yml         local override (§10.3)
    ci.env                  dummy values for config validation and the CI smoke test
    alloy/config.alloy
    prometheus/prometheus.yml
    loki/loki.yml
    grafana/provisioning/{datasources,dashboards,alerting}/…
    grafana/dashboards/{overview,infrastructure,logs}.json
.github/
  workflows/cd.yml                   (PR-A: GIT_SHA · PR-B: filters, new job)
  workflows/observability-ci.yml     (PR-B)
  workflows/uptime.yml               (PR-B)
  dependabot.yml                     (PR-B)
```

**Running the overlay.** CD always passes both files explicitly: `docker compose -f docker-compose.yml -f observability/compose.yml …`.
- `COMPOSE_FILE` is **not** set in `.env`. That avoids a circular dependency, where every compose command would fail before the overlay's files existed on the VM.
- Relative paths in the overlay are written against the project directory (`./observability/...`). That resolves the same way in the repo (`infra/`) and on the VM (`/opt/chakramcp/`).
- The overlay declares no `depends_on` on base services. An observability `up` therefore never touches the relay, Postgres, Redis or Caddy.
- For manual work, the runbook gives a shell alias for the two-file form.
- A plain `docker compose` against the base file reports the observability containers as orphans. That warning is harmless: CD never uses `--remove-orphans`.

### 9.2 CD changes (`cd.yml`)

- **Path filters.** `dorny/paths-filter` ORs patterns, so the filters use no negation.
  - `backend` becomes `backend/**` and `infra/*`: top-level infra files only, since `infra/` has no subdirectories today.
  - Dashboard or alert edits under `infra/observability/` therefore never rebuild the image or restart the relay.
  - CD needs no observability filter: the new job runs on every push (below).
- **`workflow_dispatch`.** The `which` input gains `observability`.
- **Backend job.**
  - The cargo build exports `GIT_SHA` (short sha).
  - It keeps sole ownership of syncing the base compose file and the Caddyfile, and of reloading Caddy (hash-compare, as today).
- **New job, `deploy-observability`.**
  - **When it runs:** on every push to main and on `workflow_dispatch`. It `needs` the detect job and the backend deploy job, with `if: always()` plus two conditions:
    - detect succeeded;
    - the backend job succeeded or was skipped.

    So it waits for a backend deploy in the same push, and never runs after a failed one.
  - **No path filter.** `dorny/paths-filter` only sees the commits of the push being deployed. If the concurrency group dropped an intermediate run, an observability change could otherwise stay undeployed; running every time lets the next push heal it.
  - It is cheap when nothing changed: it syncs, finds identical hashes, and touches nothing.
  - **Steps:**
    1. Sync `infra/observability/` to `/opt/chakramcp/observability/` (including deletions), recording each service's config-directory hash before and after.
    2. Create the `chakramcp_monitor` role if it doesn't exist, then `GRANT pg_monitor`.
       - It first checks, with a password-free query, whether the role exists. Only when it is missing does a second `psql` call send `CREATE ROLE … LOGIN PASSWORD …`.
       - That call runs `psql` inside the pg container, with the password read from `.env` and passed as a psql variable, never echoed.
       - So the password reaches the server only once, at creation. This matters because Postgres logs a failing statement word for word, and those logs now reach Loki.
       - Rotating the password is a manual runbook step.
    3. Run `up -d` for exactly `alloy prometheus loki grafana` with the two-file form. Compose recreates a service only when its definition or image changed.
    4. For services whose config hash changed, apply the change, **reloading instead of restarting wherever possible**, so config deploys never look like crashes (§8):

       | Service | How a config change is applied |
       |---|---|
       | Alloy, Prometheus | HTTP `POST /-/reload` (Prometheus runs with `--web.enable-lifecycle`), called from inside the Docker network |
       | Grafana dashboards | nothing: the file provider reloads them on its own |
       | Grafana alerting, datasources | Grafana's admin provisioning-reload API, called from inside the VM with the break-glass admin |
       | Loki | restart; its config rarely changes |

       A reload must be verifiable:
       - Config is bind-mounted as **directories**, never as single files. rsync replaces a changed file with a new inode, which a single-file mount would never see.
       - The job fails on a non-2xx reload response.
       - It then checks `prometheus_config_last_reload_successful` and `alloy_config_last_load_successful`, because both keep the last good config when a reload fails.
    5. Wait for health and print status.

### 9.3 One-time setup (manual; recorded in `docs/CI-CD.md`)

Every step is a no-op for the running system until the PR that needs it merges.

- **Before PR-A:** add a 2 GB swapfile (`/swapfile`, in `/etc/fstab`, `vm.swappiness=10`).
- **Before PR-B:**
  1. Create the DNS record.
  2. Add to `.env`: `GRAFANA_ADMIN_PASSWORD` and `PG_MONITOR_PASSWORD` (generated on the VM with `openssl rand`, written straight to `.env`), and `GRAFANA_ROLE_ATTRIBUTE_PATH`.
  3. Set the GitHub Actions secrets for the uptime workflow.
- **After PR-B's first green run:** make `observability-ci` a required check.

### 9.4 What each merge does in production

**PR-A merge (pick a quiet moment).** CD rebuilds and runs `up -d --force-recreate relay`.
- Postgres and Redis have diverged definitions (the new log driver), so Compose recreates them too: a few seconds of connection errors. The relay's pool reconnects on its own, and the credits worker retries.
- Caddy is not a relay dependency, so it keeps its old definition. Right after the deploy, one manual `docker compose up -d --no-deps caddy` moves it to journald and the observability network: about a 2 s blip on the TLS front door.
- Then verify:
  - `docker compose exec relay wget -qO- 127.0.0.1:9464/metrics` shows the catalogue;
  - `docker compose logs relay` shows JSON;
  - `journalctl` shows entries with the service field.

**PR-B merge.** The pre-merge steps in §9.3 are done. The merge *is* the first bring-up:
1. The backend job sees the Caddyfile change. It rebuilds, recreates the relay (no dependency changes this time), and reloads Caddy with the Grafana and metrics sites. DNS already points at the VM, so the certificate issues at once. Grafana returns 502 until the next job finishes.
2. `deploy-observability` syncs the stack, creates the monitoring role, and starts the four services.
3. Verification (§10.4).

## 10. Testing and verification

### 10.1 Backend (Rust tests, in `backend-ci`)

- **How tests record metrics:** `metrics-util`'s debugging recorder.
  - Synchronous tests scope it with `metrics::with_local_recorder`.
  - Async tests (`#[tokio::test]`, `#[sqlx::test]`) hold a `metrics::set_default_local_recorder` guard on a current-thread runtime, so parallel tests stay isolated.
- The HTTP middleware:
  - labels a templated route by its template and an unknown path `unmatched`;
  - maps a non-standard method to `other`;
  - records the status code.
- `/metrics` renders Prometheus text containing the catalogue names. No listener starts when `METRICS_ADDR` is unset.
- Each invocation terminal-write site moves `chakramcp_invocations_total` with the right `mode`, `status` and count.
  - A guarded update that changes no row records nothing.
  - `rejected` never feeds the duration histogram.
- `limits::enforce` refusal branches move `chakramcp_limit_refusals_total`, and the allowed path records nothing.
- The credits worker sets the queue-depth, stale, last-refresh and stale-window gauges.
- `LOG_FORMAT` parsing: `json`, `text`, and an unknown value that falls back to `text`.
- Request ID: a response carries `x-request-id`, and a client-supplied value is replaced.
- Panic hook: a panic is logged at ERROR with its message and location.

### 10.2 Config and smoke test (`observability-ci.yml`)

It runs on PRs that touch `infra/**` or the workflow itself. It is path-gated inside the job, so it can be a required check.

- **Static checks:**
  - `promtool check config`;
  - Loki `-verify-config`;
  - Alloy config syntax check (`alloy fmt`);
  - `docker compose -f infra/docker-compose.prod.yml -f infra/observability/compose.yml --env-file infra/observability/ci.env config -q`. `ci.env` supplies dummy values for `CHAKRAMCP_IMAGE` and every variable the files interpolate.
  - Every dashboard JSON parses and references only the provisioned datasource UIDs.
  - Alerting YAML parses.
- **Smoke test on the Linux runner**, which has journald:
  - bring up `pg`, `redis` and the four observability services with `ci.env`. `ci.env` sets `GF_UNIFIED_ALERTING_EXECUTE_ALERTS=false`, and the overlay passes it into Grafana's `environment:` as `${GF_UNIFIED_ALERTING_EXECUTE_ALERTS:-true}` (`--env-file` only feeds variable substitution). Rules then load but never try to notify.
  - create the monitoring role with the same script CD uses;
  - assert `pg_up == 1` and `redis_up == 1`. The target's own `up` isn't enough, because an exporter target reports up even when its database login fails.
  - assert one metric behind each Postgres panel (e.g. `pg_stat_database_xact_commit`, `pg_database_size_bytes`, `pg_locks_count`, `pg_postmaster_start_time_seconds`). This catches a collector list that silently drops the defaults.
  - assert that Prometheus reports the host and stack targets as up;
  - assert that Loki holds log lines with `service="pg"`, which proves the journald pipeline;
  - assert that Grafana (admin API from inside the network) lists the three dashboards, the alert rules and the Telegram contact point, without sending anything.
  - The relay and Caddy are not started, because there's no image or certificate in CI. Their scrape targets are expected to be down, and the Rust tests cover `/metrics`.

### 10.3 Local (Mac)

`infra/observability/compose.dev.yml` is an override that:
- switches every service back to the `json-file` log driver;
- resets Alloy's journal and `/etc/machine-id` mounts with Compose's `!reset` tag, since those paths don't exist on macOS;
- publishes Grafana on `127.0.0.1:3000` with the login form on and a throwaway admin password, and GitHub auth off;
- runs the relay from a locally built image;
- leaves Caddy out: start the services explicitly.

Metrics, dashboards and alert rules can be exercised locally. The log pipeline is inactive, because Docker Desktop has no journald; the CI smoke test and production cover it.

### 10.4 In production after PR-B

- every target is up in Prometheus;
- logs from every service are in Loki;
- every dashboard panel has data;
- Grafana's contact-point test delivers to Telegram;
- the operator's GitHub account signs in, and a different GitHub account is refused;
- `Authorization: Basic` from outside gets 403;
- `relay:9464` and `caddy:2020` are unreachable from the internet;
- the uptime workflow's scheduled run succeeds, and its test-message dispatch arrives.

**Performance:** a local load test against a relay endpoint with metrics on vs off shows no difference beyond noise at p50/p99. Afterwards, production relay p99 is watched for a week against the Caddy-side latency.

## 11. Rollout

0. **Swapfile** (§9.3).
1. **PR-A (backend + base compose).**
   - Contents: the metrics module, middleware and catalogue; JSON logs; request IDs; `GIT_SHA` in the CD build; `METRICS_ADDR` and `LOG_FORMAT=json` for the relay; the journald `x-logging` anchor on every base service; the `observability` network with Caddy on it.
   - Merge at a quiet moment, recreate Caddy once by hand, and verify (§9.4).
   - Nothing scrapes `/metrics` yet, so this is safe on its own. After this, `docker compose logs relay` prints JSON; pipe it through `jq` or use Grafana once it exists.
2. **PR-B pre-merge steps** (§9.3): DNS, `.env` keys, GitHub secrets.
3. **PR-B (the stack).**
   - Contents: the overlay, the dev override and `ci.env`, all configs, dashboards, alert rules, the Caddyfile changes, CD path filters and the new job, `observability-ci`, the uptime workflow, Dependabot, and the runbook in `docs/CI-CD.md`.
   - The merge brings it up (§9.4). Then verify (§10.4) and make `observability-ci` required.
4. **Credits.** Rebase #334 onto main; expect small conflicts in the credits worker, where PR-A adds metrics. Merge #334, then rebase and merge #335. The credits panels light up.

## 12. Risks

| Risk | Mitigation |
|---|---|
| Memory pressure on a 1.9 GB VM | hard caps + `GOMEMLIMIT`, swapfile, memory and OOM alerts, per-service memory panel, keep-list on self-scrapes; if it stays tight, move the stack off-box or resize the VM (a Phase 2 topology) |
| Monitoring dies with the VM | GitHub Actions uptime check; "monitoring blind" alert for a dead collector |
| Cardinality blow-up | templates only, `unmatched` route, normalised methods, bounded enums; a series-count panel |
| A compromised collector | Alloy has no Docker API, no host `/`, no capabilities, and a read-only root; it can read logs and metrics, nothing else |
| Internet-facing Grafana | GitHub-only login with an id-based allowlist in strict mode, Basic auth refused at the edge, pinned image with Dependabot patch bumps |
| Logging stalls the app | `non-blocking` log mode drops lines instead of blocking stdout |
| journald rate limit (default 10 000 messages / 30 s for `docker.service`) drops a log burst | far above normal volume at `info`; drops would show as gaps, not stalls |
| Brief blips when PR-A merges and Caddy is recreated | merged at a quiet moment; relay and worker reconnect on their own |
| Metrics and logs lost if the VM is lost | accepted: all configuration is code; history is not backed up |

## 13. Phase 2 preview (not designed here)

Packaging for self-hosters:
- the same overlay made domain-agnostic (env-driven Caddyfile hostnames; the allowlist is already a setting);
- docs for Compose;
- a Helm chart, or plain manifests, for Kubernetes that reuse the dashboard JSON and alert rules unchanged: dashboards as ConfigMaps, `ServiceMonitor`s for kube-prometheus-stack users, and Alloy as a DaemonSet reading node logs.
