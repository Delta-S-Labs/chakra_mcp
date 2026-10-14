# Self-hosting ChakraMCP

Run your own ChakraMCP network: the app API and the relay (one
`chakramcp-server` process), with Postgres and Redis, and optionally an
observability stack with metrics, logs, dashboards and alerts.

You build it from public pieces:

- **The image** `ghcr.io/delta-s-labs/chakramcp-server`, for linux/amd64
  and linux/arm64. `:latest` is the newest release (from 0.2.0 on);
  `:edge` follows `main`.
- **The Compose files in `infra/`**, which are the files production runs.
  Everything specific to a deployment lives in your `.env`.

## Choose a setup

| | For | Guide |
|---|---|---|
| **Docker Compose** | One Linux host: a VPS, a VM or a box in your VPC | [compose.md](compose.md) |
| **Kubernetes** | A cluster, with the Helm chart in `charts/chakramcp` | [kubernetes.md](kubernetes.md) |
| **A single binary** | A laptop or a quick trial (Homebrew or a source build) | [INSTALL.md](../INSTALL.md#self-hosted-server-chakramcp-server) |

Observability (ChakraMCP's dashboards and alerts on Prometheus, Loki and
Grafana) is optional on both: a stack next to Compose, or on Kubernetes
either your own kube-prometheus-stack or a bundled stack. See
[observability.md](observability.md).

## Host requirements (Compose)

- Linux with Docker Engine and the Compose plugin (v2).
- Ports 80 and 443 reachable from the internet, and DNS records for your
  hostnames pointing at the host: Caddy gets Let's Encrypt certificates
  on its own.
- **systemd with journald** for the default logging. The observability
  stack requires it: it reads container logs from the host journal.
  Docker Desktop (macOS, Windows) can run ChakraMCP with
  `LOG_DRIVER=json-file`, but not the observability stack.
- Size: production runs everything, observability included, on 2 vCPUs
  and 2 GB of RAM, using about 600 MB. Without observability, 1 GB is
  plenty.

## What's included, and what isn't

- **Sign-in is built in.** The server serves its own sign-in, consent and
  device-pairing pages, so `chakramcp login`, MCP clients, `chakramcp pair`
  and the SDKs (with keys from `chakramcp api-keys`) need nothing else.
- **Self-hosted defaults.**
  - Public sign-up is closed: you create the first admin, and anyone else,
    with `chakramcp-server users add`.
  - Credits are off, so nobody is refused for running out.
  - Both are one setting away; see [Accounts](compose.md#accounts) and
    [Credits](compose.md#credits).
- **No web dashboard.** The Next.js app in `frontend/` is the dashboard
  chakramcp.com runs; it isn't part of the self-hosted server.
- **No Postgres backups or high availability.** The Compose setup keeps
  its data in the `pgdata` volume: back it up (e.g. with `pg_dump`), or
  point `DATABASE_URL` at a managed Postgres.
