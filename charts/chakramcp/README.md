# chakramcp Helm chart

ChakraMCP (the relay and app API, one `chakramcp-server` process) on
Kubernetes, with optional bundled Postgres and Redis. The full guide, with
Ingress, TLS, secrets, GitOps and upgrades, is
[docs/self-hosting/kubernetes.md](../../docs/self-hosting/kubernetes.md).

## Install

From a release (starting with the next one):

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
| `config.frontendPublicUrl` | `appPublicUrl` | Where the web frontend lives |
| `config.adminEmail` | `""` | This account gets the server's admin role |
| `config.limitsEnforce` | `true` | `false`: over-limit calls are only logged |
| `config.discoveryV2` | `true` | |
| `config.rustLog` | `info,…,sqlx=warn` | The log filter; logs are JSON |
| `secrets.existingSecret` | `""` | A Secret whose keys all become environment variables: `JWT_SECRET` (required), plus optional ones such as `UPSERT_SHARED_SECRET`, `GITHUB_CLIENT_SECRET` or `TYPESAFE_AI_KEY`. Unset: `JWT_SECRET` is generated once and kept. |
| `extraEnv`, `extraEnvFrom` | `[]` | More environment for the server |
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

`values.schema.json` checks the values: an Ingress needs both hosts, and
turning off the bundled Postgres needs an external database.

## Secrets and GitOps

Generated secrets (`JWT_SECRET` and the bundled Postgres password) are
created once and kept: across upgrades through `lookup`, and after
`helm uninstall`, so a reinstall reuses them together with the Postgres
volume. Argo CD, Flux and `helm template` can't `lookup`, and would
generate new values on every render. With them, set
`secrets.existingSecret` and `postgresql.auth.existingSecret`.
