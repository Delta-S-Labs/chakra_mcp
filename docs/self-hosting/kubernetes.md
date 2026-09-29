# ChakraMCP on Kubernetes

The Helm chart in [`charts/chakramcp`](../../charts/chakramcp) runs
`chakramcp-server` (the app API and the relay) with an optional bundled
Postgres and Redis. Every value is listed in the
[chart README](../../charts/chakramcp/README.md).

## Install

From a checkout (the chart installs the `:edge` image):

```sh
git clone https://github.com/Delta-S-Labs/chakra_mcp
helm install chakramcp chakra_mcp/charts/chakramcp \
  --namespace chakramcp --create-namespace -f my-values.yaml
helm test chakramcp -n chakramcp
```

From the next release on, the chart is also published as an OCI artifact,
versioned with the server and defaulting to the same image version:

```sh
helm install chakramcp oci://ghcr.io/delta-s-labs/charts/chakramcp --version X.Y.Z \
  --namespace chakramcp --create-namespace -f my-values.yaml
```

Without an Ingress, reach it with
`kubectl -n chakramcp port-forward svc/chakramcp 8080:app 8090:relay`.

## A production-shaped values file

```yaml
image:
  tag: sha-abc1234        # pin a build, or a release once they exist
ingress:
  enabled: true
  className: nginx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt
  hosts:
    app: app.example.com
    relay: relay.example.com
  tls:
    - secretName: chakramcp-tls
      hosts: [app.example.com, relay.example.com]
config:
  adminEmail: you@example.com
secrets:
  existingSecret: chakramcp          # JWT_SECRET, plus optional keys
postgresql:
  enabled: false
externalDatabase:
  existingSecret: chakramcp-database  # key DATABASE_URL
```

- **Public URLs** default to the Ingress hosts, `https://` when a host has
  TLS. Set `config.appPublicUrl` and `config.relayPublicUrl` when they
  differ.
- **MCP clients** use `https://relay.example.com/mcp`.
- **Scaling.** `replicaCount` above 1 is safe: migrations take an advisory
  lock, the credits worker locks its accounting, and rate-limit counters
  live in Redis.

## Postgres and Redis

| | Bundled (default) | External |
|---|---|---|
| Postgres | One `postgres:16-alpine` pod with a volume: evaluation and small installs, no backups or failover | `postgresql.enabled=false` plus `externalDatabase.url` or `externalDatabase.existingSecret` |
| Redis | One `redis:7-alpine` pod, no persistence | `redis.enabled=false` plus `externalRedis.url`. With no Redis at all, rate limiting fails open. |

- **The bundled Postgres password** reaches the server as `PGPASSWORD`,
  never inside a URL, so any characters work.
- **An external `DATABASE_URL`** is used as given: percent-encode special
  characters in its password (`@` → `%40`, `:` → `%3A`, `/` → `%2F`,
  `#` → `%23`).

For production, use a managed Postgres (RDS, Cloud SQL, …) or an operator
such as CloudNativePG.

## Secrets

The server reads its secrets from one Secret, whose keys all become
environment variables:

| Key | |
|---|---|
| `JWT_SECRET` | Required: `openssl rand -hex 32` |
| `UPSERT_SHARED_SECRET` | Shared with the web frontend's sign-in callback; unset turns Google/GitHub sign-in off |
| `GOOGLE_CLIENT_SECRET`, `GITHUB_CLIENT_SECRET`, `RECAPTCHA_SECRET_KEY`, `TYPESAFE_AI_KEY` | Optional features; their non-secret counterparts go in `extraEnv` |

```sh
kubectl -n chakramcp create secret generic chakramcp \
  --from-literal=JWT_SECRET="$(openssl rand -hex 32)"
```

Without `secrets.existingSecret`, the chart generates `JWT_SECRET`, and the
bundled Postgres password too. Both are kept across upgrades and after
`helm uninstall`, so a reinstall reuses them with the Postgres volume.

**GitOps** (Argo CD, Flux, `helm template`): these can't read the cluster
while rendering, so generated values would change on every sync. Set
`secrets.existingSecret`, and with the bundled Postgres,
`postgresql.auth.existingSecret`.

## Upgrades and rollbacks

```sh
helm upgrade chakramcp <chart> -n chakramcp -f my-values.yaml
```

- **Migrations** run as the server starts. The startup probe allows them 5
  minutes.
- **Roll forward, don't roll back** across a release that added a
  migration. The server refuses to boot when the database has a migration
  it doesn't know, so `helm rollback` to an older image fails its probes.
  Fix forward with a newer build instead.
- **Rollouts are graceful.** The server stops accepting connections on
  SIGTERM and finishes its in-flight requests before exiting.

## Uninstalling

`helm uninstall chakramcp -n chakramcp` keeps the generated Secrets and the
bundled Postgres volume (`data-<release>-postgresql-0`). Delete them to
start over:

```sh
kubectl -n chakramcp delete secret chakramcp-secrets chakramcp-postgresql
kubectl -n chakramcp delete pvc data-chakramcp-postgresql-0
```

## Observability

The server exposes Prometheus metrics on its `metrics` port (9464) and
logs JSON to stdout. Chart pieces for Prometheus and Grafana (a
ServiceMonitor, the dashboards and the alert rules), and an optional
bundled stack, are in progress.
