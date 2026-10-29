# chakramcp Helm chart

ChakraMCP (the relay and app API, one `chakramcp-server` process) on
Kubernetes, with optional bundled Postgres and Redis, and ChakraMCP's
dashboards and alerts for your Grafana or a bundled observability stack. The full guide, with
Ingress, TLS, secrets, GitOps and upgrades, is
[docs/self-hosting/kubernetes.md](../../docs/self-hosting/kubernetes.md).

## Install

From a release (0.2.0 on):

```sh
helm install chakramcp oci://ghcr.io/delta-s-labs/charts/chakramcp --version X.Y.Z \
  --namespace chakramcp --create-namespace
```

From a checkout, which runs the `:edge` image:

```sh
helm install chakramcp charts/chakramcp --namespace chakramcp --create-namespace
```

Then `helm test chakramcp -n chakramcp`.

All pods meet the `restricted` Pod Security profile. The server runs as a
non-root user with a read-only root filesystem.

## Values

| Value | Default | Description |
|---|---|---|
| `image.repository` | `ghcr.io/delta-s-labs/chakramcp-server` | The server image |
| `image.tag` | the chart's `appVersion` | `edge` from a checkout, the release from a packaged chart. Pin a build (`sha-<7>`) or a release for anything that matters. |
| `image.pullPolicy` | `IfNotPresent` | |
| `imagePullSecrets` | `[]` | |
| `replicaCount` | `1` | More are safe: migrations and credit accounting take locks, and rate-limit counters live in Redis |
| `config.appPublicUrl`, `config.relayPublicUrl` | from the Ingress hosts | The URLs the server advertises: OAuth redirects and discovery documents |
| `config.frontendPublicUrl` | `appPublicUrl` | Where sign-in and pairing links point. The server serves its own sign-in pages at `appPublicUrl`; set this only if you run the web UI |
| `config.signupEnabled` | unset (closed) | Public sign-up. Create accounts with `chakramcp-server users add` (see `NOTES.txt`) |
| `config.creditsEnabled` | unset (off) | Credits: a monthly allowance per account. Amounts via `extraEnv` (`CREDITS_DEFAULT_MONTHLY_FREE_MC`, `CREDITS_COST_PER_INVOCATION_MC`, `CREDITS_COST_PER_MESSAGE_MC`, `CREDITS_LOW_BALANCE_MC`, `LIMITS_DEFAULT_RATE_PER_MIN`) |
| `config.adminEmail` | `""` | chakramcp.com only (`HOSTING_MODE=managed`); no effect on a self-hosted server |
| `config.limitsEnforce` | `true` | `false`: over-limit calls are only logged |
| `config.discoveryV2` | `true` | |
| `config.rustLog` | `info,…,sqlx=warn` | The log filter; logs are JSON |
| `secrets.existingSecret` | `""` | A Secret whose keys all become environment variables: `JWT_SECRET` (required), plus optional ones such as `TYPESAFE_AI_KEY`. Unset: `JWT_SECRET` is generated once and kept. |
| `extraEnv`, `extraEnvFrom` | `[]` | More environment for the server, e.g. `USAGE_RETENTION_DAYS` (days of usage events to keep, default 90; `0` keeps them forever), or the System One switches (`SYSTEM_ONE_CHECKS`, `SYSTEM_ONE_SAFETY`, with `TYPESAFE_AI_KEY` from a secret), or calls (on by default; `CALLS_ENABLED=false` turns them off; the limits `CALLS_MAX_TTL_DAYS`, `CALLS_DEFAULT_TTL_DAYS`, `CALLS_MAX_ACTIVE_PER_AGENT`, `CALLS_MAX_MESSAGES`; call memory with `CALLS_MEMORY_ENABLED`, `CALLS_TAG_THRESHOLD` and `CALLS_MEMORY_MC_PER_1K_TOKENS`, using `TYPESAFE_AI_KEY`), or MCP's public directory and agent endpoints (`MCP_AGENTS_ENABLED`, on by default with `DISCOVERY_V2`; `MCP_TOOL_WAIT_SECS`, `MCP_MAX_WAITERS`, `MCP_DIRECTORY_PER_IP_PER_MIN`; `TRUST_FORWARDED_FOR=true` when your ingress sets `X-Forwarded-For` to the client IP), or `OUTBOUND_ALLOW_PRIVATE=false` to stop the relay calling push agents on private addresses (on by default; set it off when untrusted users can register agents, since the relay pod can reach the cluster network), or the inbox pulls' longest wait and how many may wait at once (`INBOX_MAX_WAIT_SECS`, default 25; `INBOX_MAX_WAITERS`, default 2000; keep your ingress read timeout above the wait) |
| `metrics.enabled`, `metrics.port` | `true`, `9464` | Prometheus metrics on their own port, never on the Ingress |
| `service.type`, `service.appPort`, `service.relayPort` | `ClusterIP`, `8080`, `8090` | |
| `ingress.enabled`, `ingress.className`, `ingress.annotations` | `false`, `""`, `{}` | E.g. `cert-manager.io/cluster-issuer` for TLS |
| `ingress.hosts.app`, `ingress.hosts.relay` | `""` | Both required with an Ingress. The app host routes to 8080; the relay host (and MCP at `/mcp`) to 8090. |
| `ingress.tls` | `[]` | `[{secretName, hosts: […]}]` |
| `serviceAccount.create`, `.name`, `.annotations` | `true`, `""`, `{}` | The token isn't mounted |
| `podAnnotations`, `podLabels` | `{}` | |
| `podSecurityContext`, `securityContext` | restricted | uid 10001, read-only root, no capabilities, `RuntimeDefault` seccomp |
| `resources` | 50m / 64Mi requests, 512Mi limit | |
| `nodeSelector`, `tolerations`, `affinity` | empty | |
| `postgresql.enabled` | `true` | A bundled Postgres (one pod, no backups), for evaluation and small installs |
| `postgresql.image.*` | `postgres:16-alpine` | |
| `postgresql.auth.username`, `.database` | `chakramcp` | |
| `postgresql.auth.existingSecret`, `.secretKey` | `""`, `password` | The password's Secret. Unset: generated once and kept. Any characters work: the server gets it as `PGPASSWORD`. |
| `postgresql.persistence.enabled`, `.size`, `.storageClass` | `true`, `8Gi`, `""` | `enabled: false` uses an emptyDir |
| `postgresql.resources` | 100m / 128Mi requests, 1Gi limit | |
| `externalDatabase.url` | `""` | With `postgresql.enabled=false`: a `postgres://` URL, stored in a Secret. Used as given, so percent-encode its password. |
| `externalDatabase.existingSecret`, `.secretKey` | `""`, `DATABASE_URL` | Or a Secret that holds the URL |
| `redis.enabled` | `true` | A bundled Redis for rate-limit counters (no persistence, LRU-bounded) |
| `redis.image.*`, `redis.maxmemory`, `redis.resources` | `redis:7-alpine`, `128mb`, … | |
| `externalRedis.url` | `""` | With `redis.enabled=false`. Without any Redis, rate limiting fails open. |
| `observability.serviceMonitor.enabled`, `.labels`, `.interval` | `true`, `{}`, `15s` | A ServiceMonitor, rendered only when the Prometheus Operator's API exists. kube-prometheus-stack needs `labels: {release: <its release>}`. |
| `observability.dashboards.enabled`, `.labels`, `.namespace` | `true`, `grafana_dashboard: "1"`, the release's | Dashboard ConfigMaps for Grafana's sidecar |
| `observability.alertRules.enabled`, `.labels`, `.namespace` | `true`, `grafana_alert: "1"`, the release's | The Kubernetes alert rules, for Grafana's alerts sidecar |
| `observability.loki.enabled` | `false` | Your Grafana has a Loki with the chart's log labels: adds the Logs dashboard and the log rule. Implied by `bundled`. |
| `observability.datasources.prometheus`, `.loki` | `prometheus`, `loki` | Your datasources' UIDs |
| `observability.bundled.enabled` | `false` | Install Prometheus, Loki, Grafana and Alloy with the chart |
| `observability.alertChannel` | `none` | Bundled Grafana: `none`, `telegram`, `slack`, `email` or `webhook`, with settings from the Secret `chakramcp-alerts` |
| `prometheus`, `loki`, `grafana`, `alloy` | small single-node stack | The bundled subcharts' own values |

`values.schema.json` checks the values: an Ingress needs both hosts, and
turning off the bundled Postgres needs an external database.

## Secrets and GitOps

Generated secrets (`JWT_SECRET` and the bundled Postgres password) are
created once and kept: across upgrades through `lookup`, and after
`helm uninstall`, so a reinstall reuses them together with the Postgres
volume. Argo CD, Flux and `helm template` can't `lookup`, and would
generate new values on every render. With them, set
`secrets.existingSecret` and `postgresql.auth.existingSecret`.
