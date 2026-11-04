# Tracing (OpenTelemetry + Tempo) and alerts as PrometheusRule

**Status:** design, reviewed once, awaiting the user's review
**Date:** 2026-11-03
**Scope:** the backend (`chakramcp-server`), the Compose observability overlay (production and self-hosters), and the Helm chart.

## 1. Goal

Two additions to the observability stack from Phase 1 (#339/#340) and self-hosting Phase 2 (#351–#355).

1. **Alerts as a PrometheusRule.** Kubernetes users on kube-prometheus-stack should get the chart's alerts through Prometheus and Alertmanager. Today the chart ships its alerts only as Grafana alert rules, in a ConfigMap for a Grafana alerts sidecar. kube-prometheus-stack leaves that sidecar off by default, so most Kubernetes installs get no alerts.
2. **Distributed tracing.** The server exports OpenTelemetry traces over OTLP. With them, a request can be followed through its stages and outbound calls in Grafana, and a log line links to its trace.
   - chakramcp.com stores its traces in a Tempo on the VM.
   - The Compose overlay and the chart's bundled stack both include Tempo.

**Decided with the user (2026-11-03):**
- Production traces go to Tempo on the VM.
- A caller's `traceparent` is not continued. Each request starts its own trace and keeps the caller's as a span link, and nothing is forwarded to agents.
- The chart's bundled stack gets Tempo too, for parity with Compose.

**Out of scope:**
- Per-query DB spans (sqlx emits none).
- Background-worker traces (accounting, switch refresh, sweeps). Their metrics already cover them, and at their 5 s intervals they would outnumber request traces several times over.
- Metrics exemplars.
- Tempo's metrics-generator.
- Frontend (Netlify) and CLI tracing.
- Sending trace context to agents.
- Loki-ruler versions of the log alert.
- Refactoring the invoke handlers.

## 2. Constraints

| Constraint | Consequence |
|---|---|
| **Never hamper invocation performance** (standing rule). | Tracing off: no OTel layer at all. Tracing on: span bookkeeping in memory only. Export runs on the SDK's own thread through a bounded queue that drops when full. A request never waits on export, and spans add no DB queries and no I/O. |
| The VM has 1.9 GB RAM, about 720 MB available and 486 MB of swap in use (2026-11-03), and 36 GB of free disk. | Tempo gets a 192 MB `mem_limit` and a `GOMEMLIMIT` below it. It stores on local disk and keeps 7 days. |
| The repo is public, and anyone with Grafana access can read traces. | Span attributes are ids, route templates, statuses and timings. Never bodies, headers, tokens, full URLs, e-mail addresses or message text; agent endpoints and wake URLs can carry secrets in their path or query. |
| The backend deploy recreates the relay with the base Compose file only (`cd.yml`: `docker compose up -d --force-recreate relay`). | The relay's `OTEL_*` settings live in the base file, read from `.env`. The overlay can't set them. |
| Older images must keep working with the new Compose file. | The new variables default to empty. Older binaries ignore them, and the new binary treats empty as unset. |
| `charts/chakramcp/files/` is generated: `infra/observability/scripts/sync-chart-assets.sh` deletes and rewrites it, and chart-ci fails on any diff. | Nothing is hand-written under `files/`. The PromQL rules live elsewhere in the chart (§3.1). |
| `observability-ci` and `chart-ci` run main's `:edge` image, not the PR's code. | The Compose and chart Tempo PRs merge after the backend PR has rebuilt `:edge`, so their trace checks can be strict: a missing trace fails the check instead of skipping it. |

## 3. Part A: alerts as a PrometheusRule

### 3.1 The template and its source

`charts/chakramcp/templates/prometheusrule.yaml` is rendered when three things hold:
- `metrics.enabled` is on;
- `observability.prometheusRule.enabled` is on (the default);
- the `monitoring.coreos.com/v1/PrometheusRule` API exists.

This is the same gate as `servicemonitor.yaml`.

```yaml
observability:
  prometheusRule:
    # Rendered only when the monitoring.coreos.com/v1 API exists.
    enabled: true
    # kube-prometheus-stack selects rules by its release label, e.g.
    # release: kube-prometheus-stack.
    labels: {}
```

- **Where the rules live.** They're hand-written PromQL in `charts/chakramcp/alerting/prometheus-rules.yaml`, outside the generated `files/`. The template includes them under `spec.groups` with `.Files.Get`. `.helmignore` must not exclude `alerting/`.
- **What each alert carries:**
  - `severity`, the same as its Grafana rule;
  - a `uid` label equal to the Grafana rule's uid;
  - a `summary` annotation in Prometheus template syntax (`$value` where Grafana uses `$values.A`).

### 3.2 The rules

These are the chart's four metric rules (`K8S_RULES` in `sync-chart-assets.sh`), with the Grafana rules' exact thresholds:

| Alert (`uid`) | Fires when | for | severity |
|---|---|---|---|
| `ChakraMCPBackendDown` (`backend-down`) | `up{job="chakramcp"} == 0`, or `absent(up{job="chakramcp"})` | 2m | critical |
| `ChakraMCPHigh5xxRatio` (`high-5xx-ratio`) | the 5xx share of `chakramcp_http_requests_total` over 5m is > 0.05, while total traffic is > 0.1 req/s | 5m | critical |
| `ChakraMCPCreditSwitchesStale` (`credit-switches-stale`) | the Grafana rule's expression is > 0, or `absent(chakramcp_credits_switches_last_refresh_timestamp_seconds)` | 2m | critical |
| `ChakraMCPCreditQueueBacklog` (`credit-queue-backlog`) | `max(chakramcp_credits_queue_depth) > 5000` | 15m | warning |

Notes on the translation:
- **Absent arms.** The two `absent()` arms replace the Grafana rules' `noDataState: Alerting`. The switch-refresh metric always exists, because the worker runs even with credits off.
- **Probe arms.** The Grafana `backend-down` rule's `probe_success` arms are Compose-only. They never match on Kubernetes and are left out of the PromQL.
- **The log alert.** The error-log spike rule reads Loki, so it stays Grafana-only. The docs say so.
- **Duplicates.** A user whose Grafana loads the alert-rules ConfigMap and whose Prometheus selects the PrometheusRule gets each alert twice. The docs say to turn one off (`alertRules.enabled` or `prometheusRule.enabled`). Both stay on by default: on kube-prometheus-stack's defaults neither is picked up until the user opts in, and the PrometheusRule needs the `release` label, the same step the ServiceMonitor already needs.

### 3.3 Checks

- **Parity.** `sync-chart-assets.sh` (already run first in chart-ci) also reads `alerting/prometheus-rules.yaml`. It checks that every uid in `K8S_RULES` has exactly one alert with that `uid` label and that every alert names a uid in `K8S_RULES`. A drift fails chart-ci.
- **Validity and thresholds.** `promtool check rules` and **`promtool test rules`** run on the file in chart-ci, using promtool from the prom/prometheus image. The tests pin each threshold with synthetic series:
  - 5xx at 4% doesn't fire, and 6% at 0.2 req/s does;
  - no traffic doesn't fire;
  - an absent `up` fires after 2m;
  - a stale switch refresh fires;
  - a queue of 5,001 fires after 15m.
- **Rendering.** chart-ci's ci-template render adds `--api-versions monitoring.coreos.com/v1/PrometheusRule`, next to the ServiceMonitor one. Without it nothing is rendered to validate. kubeconform already has the datreeio CRD catalog.

## 4. Part B: tracing in the backend

### 4.1 Configuration

These are the standard OpenTelemetry variables, read once at startup. Empty means unset.

| Variable | Default | Meaning |
|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | Base URL of an OTLP/HTTP receiver, e.g. `http://tempo:4318`. Unset means **tracing off**: no layer, no exporter thread. Spans go to `<endpoint>/v1/traces`. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | unset | Full traces URL. It takes precedence over the base URL, as the standard says. |
| `OTEL_EXPORTER_OTLP_HEADERS` | unset | Headers for a hosted backend's auth. Never logged. |
| `OTEL_SERVICE_NAME` | `chakramcp` | The `service.name` resource attribute. |
| `OTEL_TRACES_SAMPLER` | `traceidratio` | `always_on`, `always_off` or `traceidratio`. The `parentbased_*` forms are accepted and act as their root sampler, since no remote parent is ever continued (§4.4). Any other value warns and falls back. |
| `OTEL_TRACES_SAMPLER_ARG` | `1.0` | The ratio for `traceidratio`. |

The resource also carries `service.version` (the build version, as in `chakramcp_build_info`) and `deployment.environment.name` (`managed` or `self_hosted`, from `HOSTING_MODE`).

A malformed endpoint warns at startup and leaves tracing off. It never stops the server from booting.

**`RUST_LOG` doesn't turn tracing off.** The OTel layer has its own filter (§4.2), so `RUST_LOG=warn` quiets logs and leaves traces alone.

### 4.2 The pipeline

- **Crates:** all workspace dependencies, used only from `chakramcp_shared::telemetry`.
  - `opentelemetry` 0.33;
  - `opentelemetry_sdk` 0.33, with the `trace` feature;
  - `opentelemetry-otlp` 0.33, with `http-proto` and `trace` and default features off;
  - `opentelemetry-http` 0.33;
  - `tracing-opentelemetry` 0.34.

  The SDK's `internal-logs` feature stays **off** (§4.2, failures).
- **The HTTP client.** `opentelemetry-otlp`'s own reqwest clients are reqwest 0.13, and the workspace is on 0.12. A small adapter implements `opentelemetry_http::HttpClient` with the workspace's async reqwest 0.12 client.
  - Each export runs on the server's Tokio runtime, through a `Handle` captured at init. The SDK's processor thread only waits for the result.
  - This avoids a second reqwest, and avoids a blocking client, which panics when it's built inside a Tokio context (`#[tokio::main]`, `#[tokio::test]`).
  - The request timeout equals the export timeout.
- **The batch processor.** The SDK's thread-based `BatchSpanProcessor`:
  - a queue of 2,048 spans that drops when full;
  - batches of 512, sent every 5 s;
  - a 10 s export timeout.
- **The subscriber.** `init_tracing` and `json_subscriber` move from `fmt().finish()` to `registry()` with two filtered layers:
  - **The fmt layer** keeps the `RUST_LOG` `EnvFilter`, and gets a per-layer filter that ignores spans with target `chakramcp::span` (§4.3). It still logs every event, those inside such spans included. Its current span is therefore still `http_request`, so every log line in a request keeps `request_id`, `method`, `route` and the new `trace_id`. The JSON shape doesn't change.
  - **The OTel layer** is present only when tracing is on (`Option`). Its filter takes INFO and above from the workspace's crates and `tower_http`, and **nothing** from `opentelemetry*`, `hyper*`, `reqwest*` or `h2*`, so the exporter can't trace itself.
- **Failures.** With `internal-logs` off, the SDK logs nothing. The adapter counts each failed export (an HTTP status that isn't 2xx, a timeout or a connection error) in `chakramcp_trace_export_failures_total{reason}`, and logs at most one WARN a minute saying how many failed since the last line.
  - A down Tempo therefore produces one WARN a minute, never ERROR lines. The error-log-spike alert counts only ERROR.
  - The SDK doesn't expose its drop count, so there is no dropped-spans metric.
- **Shutdown.** The SIGTERM path flushes the provider after the HTTP server has drained, capped at 1 s. That's 8 s of drain plus 1 s, inside Docker's default 10 s stop timeout.

### 4.3 What gets a span

New spans use the target `chakramcp::span`, so the fmt layer passes over them (§4.2). The invoke handlers are long (`authorize_and_enqueue_trusted` and `authorize_and_enqueue_public` are 300–400 lines each) and aren't refactored. Stage spans wrap the existing code with `.instrument(span)` on async blocks, or on the existing calls. No span guard is held across an `.await`.

| Span | Kind | Where | Attributes |
|---|---|---|---|
| `http_request` (existing; `otel.name` = `"{method} {route}"`) | server | `telemetry::instrument`'s `TraceLayer`, so both routers | `http.request.method` (normalized), `http.route` (the template), `http.response.status_code`, `request_id`, `trace_id`; `otel.status_code` = error on 5xx |
| Invocation stages | internal | A2A push and pull, `/v1/invoke` and MCP `tools/call`. The stages: authenticate, authorize (friendship, grant, consent), limits (rate limit and the credit-switch read), the safety check, then enqueue or forward | `invocation.id`, `capability.id`, `agent.caller.id`, `agent.target.id`, `invocation.mode`, the outcome |
| Outbound HTTP | client | one helper around the existing reqwest calls: push delivery and probes (`relay/src/forwarder.rs`, `relay/src/outbound.rs`), the agent-card fetcher (`relay/src/agent_card/fetcher.rs`), TypeSafe (`shared/src/system_one`), Dodo (`app/src/purchases/dodo.rs`) and wake pings | `http.request.method`, `server.address` (host only), `http.response.status_code`, `error.type`. **Never the URL path or query.** |
| Inbox pull | internal | the pull handler | `inbox.waited_secs`, `inbox.items` |

**Skipped routes.** `instrument()` gains a skip for `/healthz`, which is new work since it skips nothing today. `/metrics` is on its own listener and isn't instrumented.

The safety check's TypeSafe call and the push delivery are the spans most worth having: they're the slow, external parts of an invocation.

### 4.4 Trace context and the response header

- **Inbound.** A W3C `traceparent` (and `tracestate`) on a request is parsed but **never** used as the parent.
  - The root span starts a new trace under our own sampler and adds a span link to the caller's context (`link.source` = `caller`).
  - So callers can't force sampling, flood the trace store, or attach our spans to their traces. Agent developers can still find the relay's trace from theirs through the link.
  - A malformed header is ignored.
- **Outbound.** No `traceparent` is sent anywhere: not to push agents, wake URLs, agent-card hosts, TypeSafe or Dodo.
- **`ChakraMCP-Trace-Id`.** When a request's trace is **sampled**, the response carries this header with the 32-hex trace id. Unsampled requests and tracing off mean no header, since an unsampled id leads nowhere.
  - It's for operators and support, and the Compose and chart checks use it to find a request's trace.
  - The id carries nothing secret; callers already get `request_id`.

### 4.5 Logs and traces

When tracing is on, the request span records `trace_id` as soon as the OTel context exists. The JSON format includes the current span's fields, and the fmt layer's current span is always `http_request` (§4.2), so every JSON log line inside a request carries `trace_id`. Text logs are unchanged. With tracing off, the field stays empty and isn't written.

- **Loki → Tempo.** Grafana's Loki data source gets a derived field (`"trace_id":"(\w{32})"`) that links to Tempo.
- **Tempo → Loki.** Tempo's data source gets `tracesToLogsV2`, querying `{service="relay"} |= "<trace_id>"` over the span's time range ±1 m.

### 4.6 Performance

- **Off:** no layer, no thread, no allocation. The `trace_id` field is declared empty, like `request_id`.
- **On:** each span costs the `tracing-opentelemetry` bookkeeping, a few allocations and microseconds. Unsampled spans still pay part of that. Export happens off the request path, and the bounded queue drops instead of blocking.
- **Measured before production turns it on.** No load test is committed, so the plan picks the tool and the rate. The measurement covers `/v1/invoke` on the local stack, push mode to a local echo agent, with tracing off and on.
  - The off case must match `main` within noise.
  - The on case's overhead goes in the PR description.

## 5. Part C: Tempo in the Compose overlay (production and self-hosters)

### 5.1 The service

`infra/observability/compose.yml` gains `tempo`:
- the image `grafana/tempo:3.1.1`, pinned like the others;
- `mem_limit: 192m` and `GOMEMLIMIT=160MiB`;
- `cap_drop: [ALL]` and `read_only: true`;
- the volume `tempo_data` at `/var/tempo`;
- the shared journald logging;
- **both** networks, `internal` (the relay reaches `tempo:4318`) and `observability` (Grafana and Alloy reach it), with nothing published to the host.

The config is `infra/observability/tempo/tempo.yml`, Tempo 3's single-binary mode. It needs no Kafka: the distributor pushes straight to the live-store.
- The OTLP receiver is HTTP only, bound explicitly to `0.0.0.0:4318`. Recent Tempo versions bind receivers to localhost otherwise.
- Local storage under `/var/tempo`.
- Block retention of 168h.
- `usage_report` off, and `multitenancy_enabled: false`.

### 5.2 Wiring

- **The base Compose file.** `infra/docker-compose.prod.yml`'s relay gets `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME`, `OTEL_TRACES_SAMPLER` and `OTEL_TRACES_SAMPLER_ARG`, each as `${VAR:-}`. Tracing stays off until `.env` sets the endpoint. `docs/self-hosting/compose.md` says that with the observability overlay you set `OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4318`.
- **Grafana.** `datasources.yml` gains Tempo (`uid: tempo`, `url: http://tempo:3200`, with `tracesToLogsV2` to `loki`). Loki gains `derivedFields` for `trace_id`. `check-grafana.py`'s `DATASOURCE_UIDS` gains `tempo`.
- **Alloy.** Tempo's `/metrics` joins the existing `stack` scrape, as job `tempo`. Its allowlist already keeps `up` and `process_*`.
- **Alerts:**
  - `crash-loop`'s job list (`caddy|prometheus|loki|grafana|integrations/self`) gains `tempo`;
  - a new warning rule, **"Trace export failing"**: `increase(chakramcp_trace_export_failures_total[15m]) > 0` for 15m. This is how a dead or full Tempo shows up, since its traces are lost without errors. It's a Compose rule only; the chart's `K8S_RULES` doesn't change.
- **Deploying it:**
  - `cd.yml`'s deploy-observability job hashes `tempo` along with the other config directories (`for d in alloy prometheus loki …`).
  - `deploy.sh` gains `tempo` in its `obs up -d …`, `wait_for` and `obs ps` lines, and a `has tempo` branch. Tempo can't reload, so that branch recreates it (`obs up -d --force-recreate tempo`).
  - observability-ci passes `tempo` in its `deploy.sh` directory list.

### 5.3 Production rollout

1. The backend PR merges and deploys. Tracing stays off, since `.env` has no endpoint.
2. The overlay PR merges, and CD's deploy-observability starts Tempo. Still no traces.
3. Check Tempo's memory for a day (`docker stats`).
4. Append `OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4318` to `/opt/chakramcp/.env`. It's one line and not a secret, and it needs the user's OK at the time. Then run `docker compose up -d --force-recreate relay`.
5. Check in Grafana: an invoke has a trace with its stage and client spans, a log line links to the trace, and the trace links back to its logs.

### 5.4 Sampling in production

Production samples 100% (ratio 1.0). With no background traces, the volume is:
- the house agents' waiting pulls, about 7,000 short traces a day;
- real traffic, well under 100k spans a day in all, which is a few MB a day in Tempo.

If it grows, `.env` lowers `OTEL_TRACES_SAMPLER_ARG`.

## 6. Part D: Tempo in the chart's bundled stack

### 6.1 The subchart

`Chart.yaml` gains:

```yaml
  - name: tempo
    version: 3.2.1          # appVersion 3.1.1, single binary
    repository: oci://ghcr.io/grafana-community/helm-charts
    condition: observability.bundled.enabled
```

`values.yaml` gets a `tempo:` block, written like the Loki one:
- one replica, 4Gi of persistence, `tempo.retention: 168h`;
- the OTLP HTTP receiver only, on `0.0.0.0:4318`;
- `reportingEnabled: false`;
- the restricted Pod Security profile:
  - `runAsNonRoot`, uid and gid 10001;
  - `seccompProfile: RuntimeDefault`;
  - `allowPrivilegeEscalation: false`, `capabilities.drop: [ALL]` and `readOnlyRootFilesystem`;
- `tempoQuery.enabled: false`;
- resource requests of 50m CPU and 128Mi memory, with a 256Mi memory limit.

The bundled Alloy doesn't scrape Tempo: nothing in the chart would read those metrics.

### 6.2 Wiring

- **The server's settings.**
  ```yaml
  tracing:
    # An OTLP/HTTP endpoint for the server's traces. Empty: the bundled
    # Tempo when observability.bundled.enabled, else tracing off.
    endpoint: ""
    sampler: ""        # always_on | always_off | traceidratio
    samplerArg: ""     # e.g. "0.1"
    headersSecret: ""  # a Secret with key OTEL_EXPORTER_OTLP_HEADERS
  ```
  - **The endpoint.** The deployment sets `OTEL_EXPORTER_OTLP_ENDPOINT` to `tracing.endpoint`. When that's empty and the bundled stack is on, it uses the bundled Tempo's Service, named with the chart's `chakramcp.subchartFullname` helper, on port 4318.
  - **The rest.** It sets the sampler variables when they're non-empty, and loads `OTEL_EXPORTER_OTLP_HEADERS` from `headersSecret`.
  - **The schema.** `values.schema.json` covers the new keys.
- **The bundled Grafana.** Its datasources value gains Tempo (`uid: tempo`) with `tracesToLogsV2`, and Loki gains the `trace_id` derived field, mirroring Compose. The Tempo URL follows the same release-name pattern as Loki's.
- **Existing stacks** (bundled off). No Tempo is installed. The docs point `tracing.endpoint` at the user's own collector or Tempo, and give the data-source settings for both links.

### 6.3 Chart tests

- `ci/bundled-values.yaml` installs Tempo on kind, in the restricted namespace as today.
- `templates/tests/test-observability.yaml` gains a trace check:
  1. make one request to the server;
  2. read `ChakraMCP-Trace-Id`;
  3. poll Tempo's `/api/v2/traces/<id>` until the trace is found, for up to 60 s.

  It's strict: this PR merges after the backend PR has rebuilt `:edge`, the image chart-ci installs.
- Lint, the render and kubeconform cover the new manifests.

## 7. Testing

### 7.1 Backend (`backend-ci`)

- **Unit tests:**
  - config parsing: empty means unset, a malformed endpoint warns and leaves tracing off, unknown samplers fall back, and `RUST_LOG=warn` keeps the OTel layer;
  - the client-span helper never records a URL path or query;
  - the adapter's failure counter and its once-a-minute WARN.
- **`shared/tests/tracing.rs`.** Its own test binary, since it installs a global subscriber. A fake OTLP/HTTP receiver (axum) decodes `ExportTraceServiceRequest`, and requests go through a router built with `instrument()` and tracing on. The test checks:
  - a server span with the route template and status;
  - a request with `traceparent` gets a **new** trace id and a link to the caller's;
  - a 5xx marks the span as an error;
  - `ChakraMCP-Trace-Id` matches the exported trace when sampled, and is absent under `always_off`;
  - no exported attribute contains a `?` query or a URL path;
  - `/healthz` produces no span.
- **`shared/tests/json_logs.rs`** gains three cases:
  - with tracing on, a line logged in the request span carries `trace_id`, equal to the exported span's;
  - a line logged inside a nested `chakramcp::span` span still carries `request_id`, `route` and `trace_id`;
  - with tracing off, there's no `trace_id`.
- **`relay/tests/tracing.rs`.** A new integration binary, kept apart from the in-crate tests to avoid the callsite-interest flakiness `json_logs.rs` describes. It uses the fake receiver, exported from `chakramcp_shared` behind the existing `test-support` feature, and checks two things:
  - one push and one pull invoke produce their stage spans under the request span;
  - the push delivery reaches a fake agent **without** a `traceparent` header.

### 7.2 Compose (`observability-ci`)

- **Config checks.** `tempo -config.verify` runs on `tempo.yml`, next to the Prometheus and Loki checks.
- **The env.** `infra/observability/ci.env` sets `OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4318`.
- **`smoke-test.py`** checks:
  - Tempo's `/ready` is OK;
  - the Grafana Tempo data source's health check passes;
  - one request's trace, found by `ChakraMCP-Trace-Id`, arrives in Tempo within 60 s. This is strict (§2): a missing header fails.

### 7.3 Chart (`chart-ci`)

See §3.3 (parity, promtool check and test, the PrometheusRule render, kubeconform) and §6.3 (the kind install and the trace check).

### 7.4 Local and production

- **Local, on colima:** the overlay with the endpoint set, one invoke viewed in Grafana, and screenshots for the PR.
- **The measurement** in §4.6.
- **Production:** the rollout in §5.3.

## 8. Docs

- **`docs/self-hosting/observability.md`** gets two sections.
  - **Tracing:**
    - the variables;
    - Compose (set the endpoint) and the chart (`tracing.*` and the bundled Tempo);
    - sampling;
    - what spans carry and what they never carry;
    - the caller link;
    - `ChakraMCP-Trace-Id`;
    - that `RUST_LOG` doesn't affect it.
  - **PrometheusRule:**
    - the `release` label;
    - the four alerts;
    - that the log alert is Grafana-only;
    - how to avoid duplicates.
- **The chart README:** the new values.
- **`docs/self-hosting/compose.md`:** the endpoint line in the observability step.
- **The agent-facing docs** (`/docs` and llms.txt): one line saying that a request's `traceparent` is kept as a link, and that `ChakraMCP-Trace-Id` comes back when the operator traces. Check it against the Guide's 150 KB bundle cap.

## 9. Rollout

| Order | PR | Production effect on merge |
|---|---|---|
| any | **1.** The PrometheusRule template and rules, the sync-script parity check, promtool check and test, the render flag, docs | none (chart only) |
| first of 2–4 | **2.** Backend tracing: config, pipeline, adapter, spans, links, `trace_id` in logs, `ChakraMCP-Trace-Id`, the failure metric, tests, the base Compose env lines, docs | the relay is redeployed with tracing off |
| after 2, once `:edge` is rebuilt | **3.** Tempo in the Compose overlay: Grafana data source and links, the Alloy scrape, crash-loop coverage, the export-failing rule, cd.yml and deploy.sh, observability-ci | Tempo starts on the VM; no traces yet |
| — | Append the endpoint to the VM `.env` and recreate the relay (with the user's OK) | traces flow |
| after 2, once `:edge` is rebuilt | **4.** Tempo in the chart's bundled stack, the `tracing.*` values, the chart test | none (chart only) |

All of it ships in the next release (0.8.0), and each PR's notes go on the next-release list as it merges.

## 10. Risks

| Risk | Mitigation |
|---|---|
| Tempo's memory, on a VM already using swap. | A 192 MB hard limit, `GOMEMLIMIT` and low traffic. The memory alert already exists. Tempo runs for a day before the endpoint is set. If it doesn't fit: lower the retention or the sampling, or remove the endpoint (one `.env` line). |
| The exporter floods logs while Tempo is down. | SDK internal logs are off. The adapter logs one WARN a minute and counts the failures, the OTel layer ignores the exporter's own targets, and "Trace export failing" alerts. |
| A secret lands in a span (a URL path or query, a header). | Client spans record the host only. A test asserts no path or query in the exported attributes. Bodies and headers are never recorded. Review checks every new `fields(...)`. |
| Callers control the trace volume. | Inbound context is only a link, sampling is ours, and the queue is bounded. |
| The nested spans change the JSON logs. | The fmt layer ignores `chakramcp::span` spans, and `json_logs.rs` covers a line logged inside one. |
| `tracing-opentelemetry` overhead on the invocation path. | Off means no layer at all. On is measured (§4.6) before production turns it on. |
| Tempo 3 config drift (3.0 changed the architecture). | Pin 3.1.1, run `-config.verify` in CI, and use single-binary mode, which is documented for 3.x without Kafka. |
| The subchart's fit with the restricted Pod Security profile. | Tested on kind in a namespace labelled `pod-security.kubernetes.io/enforce=restricted`, like the other subcharts. |
