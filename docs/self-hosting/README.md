# Self-hosting ChakraMCP

Run your own ChakraMCP network: the app API and the relay (one
`chakramcp-server` process), with Postgres and Redis, and optionally an
observability stack with metrics, logs, dashboards and alerts.

You build it from public pieces:

- **The image** `ghcr.io/delta-s-labs/chakramcp-server`, for linux/amd64
  and linux/arm64. `:edge` follows `main`; versioned tags arrive with the
  next release.
- **The Compose files in `infra/`**, which are the files production runs.
  Everything specific to a deployment lives in your `.env`.

## Choose a setup

| | For | Guide |
|---|---|---|
| **Docker Compose** | One Linux host: a VPS, a VM or a box in your VPC | [compose.md](compose.md) |
| **Kubernetes** | A cluster, with a Helm chart | In progress |
| **A single binary** | A laptop or a quick trial (Homebrew or a source build) | [INSTALL.md](../INSTALL.md#self-hosted-server-chakramcp-server) |

Observability (Prometheus, Loki, Grafana and Alloy, with ChakraMCP's
dashboards and alerts) is an optional layer on the Compose setup:
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

## What's not included

- **The web UI.** The Next.js frontend (`frontend/`) runs separately
  from the server; the CLI, the SDKs and MCP clients work without it.
- **Backups and high availability** for Postgres. The Compose setup
  keeps its data in the `pgdata` volume: back it up (e.g. with
  `pg_dump`), or point `DATABASE_URL` at a managed Postgres.
