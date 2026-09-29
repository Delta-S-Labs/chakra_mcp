# Observability for self-hosters

`infra/observability/` adds four services next to the
[Compose setup](compose.md):

- **Alloy** collects metrics from ChakraMCP, Caddy, Postgres, Redis and
  the host, and reads every container's logs from the host journal.
- **Prometheus** keeps metrics for 15 days (2 GB at most).
- **Loki** keeps logs for 14 days.
- **Grafana** serves the dashboards and evaluates the alert rules.

You get three dashboards (**Overview**, **Infrastructure** and **Logs**)
and twelve alert rules. They're the same files production runs.

## Requirements

- A **systemd Linux host** with Docker's journald log driver
  (`LOG_DRIVER=journald`, the default) and **persistent journal storage**
  (`/var/log/journal`, the default on Ubuntu and Debian). Alloy reads
  container logs from the journal. Docker Desktop can't run the stack.
- About 1.2 GB of memory for the four services at their limits.

`deploy.sh` checks the host before it changes anything and says what's
missing. If the journal is volatile, enabling persistent storage takes
`sudo mkdir -p /var/log/journal && sudo systemctl restart systemd-journald`.

## Setup

1. **DNS** for Grafana's hostname, e.g. `grafana.example.com`.

2. **`.env`** (see `infra/.env.example`):
   - `GRAFANA_DOMAIN`: that hostname;
   - `GRAFANA_ADMIN_PASSWORD`: the `admin` login;
   - `PG_MONITOR_PASSWORD`: the collector's read-only Postgres role.

   After setting `GRAFANA_DOMAIN`, apply it to Caddy:
   `docker compose -f docker-compose.prod.yml up -d caddy`.

3. **Deploy**, from `infra/`:

   ```sh
   observability/scripts/deploy.sh "alloy prometheus loki grafana/provisioning grafana/dashboards grafana/channels"
   ```

   The script:
   - checks the host;
   - creates the Postgres monitoring role;
   - starts the stack;
   - applies the configs named in its argument;
   - verifies that Prometheus and Alloy loaded them.

   CD runs the same script in production.

4. **Sign in** at `https://grafana.example.com` as `admin`.

## Signing in with a provider

Grafana starts with only the `admin` form login. You can add GitHub,
Google or any OpenID Connect provider, in `.env`:

| Provider | Settings | Callback URL |
|---|---|---|
| GitHub | `GRAFANA_GITHUB_ENABLED=true`, `GRAFANA_GITHUB_CLIENT_ID`, `GRAFANA_GITHUB_CLIENT_SECRET` (an OAuth app); optionally `GRAFANA_GITHUB_AUTO_LOGIN=true` | `https://<GRAFANA_DOMAIN>/login/github` |
| Google | `GRAFANA_GOOGLE_ENABLED=true`, `GRAFANA_GOOGLE_CLIENT_ID`, `GRAFANA_GOOGLE_CLIENT_SECRET` | `https://<GRAFANA_DOMAIN>/login/google` |
| OpenID Connect | `GRAFANA_OIDC_ENABLED=true`, `GRAFANA_OIDC_NAME`, `…_CLIENT_ID`, `…_CLIENT_SECRET`, `…_AUTH_URL`, `…_TOKEN_URL`, `…_API_URL`; scopes default to `openid email profile` | `https://<GRAFANA_DOMAIN>/login/generic_oauth` |

**Who may sign in** is `GRAFANA_ROLE_ATTRIBUTE_PATH`, a
[JMESPath](https://jmespath.org) expression over the provider's user
info that yields a Grafana role. Anyone it yields nothing for is
refused, so an empty value refuses everyone.

- GitHub (its `/user` response):
  `GRAFANA_ROLE_ATTRIBUTE_PATH="login == 'your-login' && 'GrafanaAdmin' || ''"`.
  The numeric `id` is safer than the login, which can change; it's a
  number literal: `` id == `12345` ``.
- Google or OIDC (the token's claims):
  `GRAFANA_ROLE_ATTRIBUTE_PATH="email == 'you@example.com' && 'GrafanaAdmin' || ''"`.
- Several people: `"contains(['alice', 'bob'], login) && 'Editor' || ''"`.

Once a provider works, `GRAFANA_DISABLE_LOGIN_FORM=true` hides the admin
form. Password (Basic) authentication is always refused from outside by
Caddy, so the `admin` account then works only from inside the host.

After changing any of these, run `deploy.sh` again. Compose recreates
Grafana with the new settings.

## Alerts

`ALERT_CHANNEL` picks where alerts go:

| `ALERT_CHANNEL` | Settings |
|---|---|
| `none` (default) | None: alerts show in Grafana only |
| `telegram` | `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID` |
| `slack` | `SLACK_WEBHOOK_URL` (an incoming webhook) |
| `email` | `ALERT_EMAIL_TO`, plus an SMTP server: `GF_SMTP_ENABLED=true`, `GF_SMTP_HOST`, `GF_SMTP_USER`, `GF_SMTP_PASSWORD`, `GF_SMTP_FROM_ADDRESS` |
| `webhook` | `ALERT_WEBHOOK_URL`: Grafana POSTs its alert JSON |

Switch by editing `.env` and running `deploy.sh` again. The previous
channel's contact point is removed.

The rules cover: the backend or Postgres down, a high 5xx ratio, an
expiring certificate, credit enforcement falling behind, the disk or
memory running out, OOM kills, crash loops, error-log spikes, and the
monitoring itself going blind.

## HTTPS probes

`PROBE_TARGETS` lists URLs to probe from the host every minute, as JSON.
Single-quote it in `.env`:

```sh
PROBE_TARGETS='[{"name":"relay","address":"https://relay.example.com/healthz"},{"name":"app","address":"https://app.example.com/healthz"}]'
```

- Probes are HTTPS-only.
- They feed the Overview's **Probes** panel and the certificate-expiry
  alert.
- The **Backend down** alert watches the names `relay` and `app`.
  Without probes, it relies on the metrics scrape alone.

## Upgrades

After `git pull` (see [compose.md](compose.md#upgrades)), run `deploy.sh`
again with the directories that changed. Passing them all is safe: the
script reloads rather than restarts where it can.

## Operating

The stack needs both compose files on every command:

```sh
alias obs='docker compose -f docker-compose.prod.yml -f observability/compose.yml'
obs ps alloy prometheus loki grafana
obs logs --tail 50 grafana
```

- A plain `docker compose -f docker-compose.prod.yml …` lists the
  observability containers as orphans. That's harmless; don't use
  `--remove-orphans`.
- **Dashboards and alert rules** are files in
  `infra/observability/grafana/`. Change the files rather than the UI,
  then re-run `deploy.sh`.
- **Prometheus, Loki and Alloy aren't published.** To reach one, tunnel
  to its container address, e.g. `ssh -L 9090:<ip>:9090 host`, where
  `<ip>` comes from `docker inspect` on the container.
- **Rotating `PG_MONITOR_PASSWORD`:**
  1. Change it in `.env`.
  2. Run `obs exec -T pg psql -U chakramcp -d chakramcp -c "ALTER ROLE chakramcp_monitor PASSWORD '…'"`.
  3. Run `obs up -d --force-recreate alloy`.
