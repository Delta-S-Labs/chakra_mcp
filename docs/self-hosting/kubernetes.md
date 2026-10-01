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

Each release (from 0.2.0 on) is also published as an OCI chart, versioned
with the server and defaulting to the same image version:

```sh
helm install chakramcp oci://ghcr.io/delta-s-labs/charts/chakramcp --version X.Y.Z \
  --namespace chakramcp --create-namespace -f my-values.yaml
```

Without an Ingress, reach it with
`kubectl -n chakramcp port-forward svc/chakramcp 8080:app 8090:relay`.

Then create your account. Public sign-up is closed, so the first admin is
made from inside the cluster (`helm install` prints this command with your
release's name):

```sh
kubectl -n chakramcp exec -it deploy/chakramcp -- \
    chakramcp-server users add you@example.com --name "Your Name" --admin
```

To pipe the password in instead, use `exec -i` and `--password-stdin`.
Then sign in with the CLI (`chakramcp networks add`, `networks use`,
`login`) and connect MCP clients, as in
[compose.md](compose.md#first-start) from step 6. The server serves its
own sign-in, consent and device-pairing pages.

## A production-shaped values file

```yaml
image:
  tag: "0.2.0"            # pin a release (a packaged chart defaults to its own)
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
  signupEnabled: false   # the default; true lets people create accounts
  creditsEnabled: false  # the default; true gives accounts a monthly allowance
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

## Accounts and credits

These work as on Compose. [compose.md](compose.md#accounts) has the
details: who can sign up, the admin role, the failed sign-in limit, and
turning credits on. Run the commands with
`kubectl -n chakramcp exec deploy/chakramcp -- chakramcp-server …`, e.g.
`… users list` or `… credits show you@example.com`.

- `config.signupEnabled` and `config.creditsEnabled` set the switches.
  Credit amounts go through `extraEnv`: `CREDITS_DEFAULT_MONTHLY_FREE_MC`,
  `CREDITS_COST_PER_INVOCATION_MC`, `LIMITS_DEFAULT_RATE_PER_MIN`.
- `config.adminEmail` has no effect on a self-hosted server.

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
| `TYPESAFE_AI_KEY` | Optional: System One compliance checks, with `SYSTEM_ONE_CHECKS=true` in `extraEnv` |

```sh
kubectl -n chakramcp create secret generic chakramcp \
  --from-literal=JWT_SECRET="$(openssl rand -hex 32)"
```

Without `secrets.existingSecret`, the chart generates `JWT_SECRET`, and the
bundled Postgres password too. Both are kept across upgrades and after
`helm uninstall`, so a reinstall reuses them with the Postgres volume.

**GitOps** (Argo CD, Flux, `helm template`): these can't read the cluster
while rendering, so generated values would change on every sync. Set
`secrets.existingSecret`; with the bundled Postgres,
`postgresql.auth.existingSecret`; and with the bundled observability stack,
`grafana.admin.existingSecret`.

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

The chart ships ChakraMCP's dashboards and alert rules. With your own
Prometheus and Grafana (e.g. kube-prometheus-stack), that's a
ServiceMonitor plus ConfigMaps for Grafana's sidecars. Alternatively,
`observability.bundled.enabled=true` installs a small Prometheus, Loki,
Grafana and Alloy stack. Both are in
[observability.md](observability.md#kubernetes).
