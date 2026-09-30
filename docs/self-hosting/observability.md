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

## Kubernetes

The Helm chart ([kubernetes.md](kubernetes.md)) ships the same dashboards
and alert rules, in one of two ways.

### With your own Prometheus and Grafana (the default)

For kube-prometheus-stack or anything like it, the chart renders:

- **A ServiceMonitor**, when the Prometheus Operator's API exists. It
  scrapes the metrics port as `job="chakramcp"` and keeps the metrics' own
  `service` label (`honorLabels`). kube-prometheus-stack only selects
  ServiceMonitors that carry its release label:

  ```yaml
  observability:
    serviceMonitor:
      labels:
        release: kube-prometheus-stack   # your kube-prometheus-stack release name
  ```

  Without an operator, scrape port 9464 of the chart's Service yourself,
  as `job="chakramcp"`.
- **Dashboards**: ConfigMaps labelled `grafana_dashboard: "1"`, which
  kube-prometheus-stack's Grafana sidecar loads from every namespace. You
  get the Overview; with `observability.loki.enabled`, also the Logs
  dashboard and the Overview's log panel. The Infrastructure dashboard is
  Compose-only; on Kubernetes, the cluster's own dashboards cover nodes
  and pods.
- **Alert rules**: a ConfigMap labelled `grafana_alert: "1"`.
  kube-prometheus-stack's alerts sidecar is **off** by default and only
  watches Grafana's namespace, so turn it on and let it see the rules:

  ```yaml
  # kube-prometheus-stack values
  grafana:
    sidecar:
      alerts:
        enabled: true
        searchNamespace: ALL
  ```

  Alternatively, set `observability.alertRules.namespace` to Grafana's
  namespace. Your Grafana's notification policies route the alerts.

Only the rules that work on Kubernetes are shipped:
- backend down;
- a high 5xx ratio;
- the credit switches going stale;
- the credit queue backing up;
- with Loki, the error-log spike.

The host, Postgres and certificate rules need Compose's exporters and
probes.

Datasource UIDs default to `prometheus` (kube-prometheus-stack's) and
`loki`. Set `observability.datasources.*` if yours differ.
`observability.loki.enabled` expects a Loki whose streams carry the
chart's `service` (the pod's `app.kubernetes.io/component`) and `level`
labels, as the bundled stack's do.

### Bundled

`observability.bundled.enabled=true` installs a small stack next to the
server:
- Prometheus (server only, 15 days);
- Loki (single binary, 14 days);
- Grafana, with the dashboards and alert rules;
- Alloy (one replica), which scrapes the server and tails the release's
  pod logs through the Kubernetes API.

Every pod meets the `restricted` Pod Security profile.

```sh
kubectl -n chakramcp port-forward svc/chakramcp-grafana 3000:80
kubectl -n chakramcp get secret chakramcp-grafana -o jsonpath='{.data.admin-password}' | base64 -d
```

- **Alerts** go to `observability.alertChannel`: `none`, `telegram`,
  `slack`, `email` or `webhook`, with the same settings as Compose, in a
  Secret named `chakramcp-alerts`:

  ```sh
  kubectl -n chakramcp create secret generic chakramcp-alerts \
    --from-literal=TELEGRAM_BOT_TOKEN=… --from-literal=TELEGRAM_CHAT_ID=…
  ```

  For email, also set `grafana.smtp` or the `[smtp]` section in
  `grafana."grafana.ini"`.
- **Sign-in** is the admin by default. Providers are Grafana settings, as
  in Compose, e.g. GitHub:

  ```yaml
  grafana:
    grafana.ini:
      server:
        root_url: https://grafana.example.com
      auth.github:
        enabled: true
        client_id: …
        scopes: user:email,read:org
        allow_sign_up: true
        role_attribute_path: "login == 'your-login' && 'GrafanaAdmin' || ''"
        role_attribute_strict: true
        allow_assign_grafana_admin: true
    envFromSecrets:
      - name: chakramcp-alerts
        optional: true
      - name: chakramcp-grafana-auth   # GF_AUTH_GITHUB_CLIENT_SECRET=…
  ```

  Expose Grafana with the subchart's own `grafana.ingress`.
- **GitOps**: also set `grafana.admin.existingSecret`, next to the chart's
  other existing Secrets.
- **Every subchart setting** stays available under `prometheus`, `loki`,
  `grafana` and `alloy`, e.g. volume sizes and retention.
