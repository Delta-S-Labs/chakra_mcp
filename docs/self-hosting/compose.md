# ChakraMCP on Docker Compose

`infra/docker-compose.prod.yml` runs the whole server on one host:

| Service | What |
|---|---|
| `caddy` | TLS and the reverse proxy on ports 80/443 |
| `relay` | `chakramcp-server`: the app API (8080) and the relay (8090) |
| `pg` | Postgres 16, in the `pgdata` volume |
| `redis` | Rate-limit counters (no persistence) |

Production runs this same file; its settings are in its own `.env`. See
[the requirements](README.md#host-requirements-compose) first.

## First start

1. **DNS.** Create A (or AAAA) records for two hostnames, e.g.
   `app.example.com` and `relay.example.com`, pointing at the host.

2. **Get the files.** Everything runs from the checkout's `infra/`:

   ```sh
   git clone https://github.com/Delta-S-Labs/chakra_mcp
   cd chakra_mcp/infra
   ```

3. **Configure.** Copy the template and fill in the required section:

   ```sh
   cp .env.example .env
   chmod 600 .env
   ```

   - `APP_DOMAIN` and `RELAY_DOMAIN`: the hostnames from step 1.
   - `APP_PUBLIC_URL` and `RELAY_PUBLIC_URL`: the same, as `https://` URLs.
   - `POSTGRES_PASSWORD` and `JWT_SECRET`: from `openssl rand -hex 32`,
     one each.

   Everything else is optional and documented in the file.

4. **Start.**

   ```sh
   docker compose -f docker-compose.prod.yml up -d
   docker compose -f docker-compose.prod.yml ps
   ```

   The relay applies database migrations as it boots. Caddy requests the
   certificates within a minute or so. Check the result with
   `curl https://relay.example.com/healthz`.

5. **Create your account.** Public sign-up is closed, so the first admin
   is made from the host. You're asked for a password twice:

   ```sh
   docker compose -f docker-compose.prod.yml exec relay \
       chakramcp-server users add you@example.com --name "Your Name" --admin
   ```

   To pipe the password in instead, use `exec -T` and `--password-stdin`.

6. **Sign in with the CLI.** Add your network, switch to it (a fresh CLI
   points at the public network), and sign in. Your browser opens the
   server's own sign-in page:

   ```sh
   chakramcp networks add private \
       --app-url https://app.example.com \
       --relay-url https://relay.example.com
   chakramcp networks use private
   chakramcp login
   ```

   On a machine without a browser, `chakramcp pair` prints a link and a
   code to approve from another device.

7. **Connect MCP clients** to `https://relay.example.com/mcp`. Clients that
   sign in with OAuth use the same sign-in page; for the others, and for
   the SDKs, create an API key:

   ```sh
   chakramcp api-keys create --name laptop
   ```

To add teammates, see [Accounts](#accounts). To add dashboards and alerts,
continue with [observability.md](observability.md).

## Accounts

The server serves its own sign-in, consent and device-pairing pages, so
nothing else needs to run. Admins and passwords are managed from the host
with `chakramcp-server users`:

| Command | |
|---|---|
| `users add <email> --name <name> [--admin]` | Create an account. `--admin` gives the admin role. |
| `users list [--json]` | Everyone, with each personal account's slug |
| `users set-password <email>` | There's no email reset, so this is how a forgotten password is replaced |
| `users set-admin <email> [--off]` | Takes effect at the user's next sign-in |

Prefix each with
`docker compose -f docker-compose.prod.yml exec relay chakramcp-server`.

- **Sign-up.** Closed by default. Set `SIGNUP_ENABLED=true` in `.env` and
  run `up -d` again to let people create their own accounts; the sign-in
  page then links to `/signup`. Close it again once your team is in.
- **The admin role** opens the admin API (`/v1/admin/*`) to a token from
  `POST /v1/auth/login`. Day to day, the commands here and under
  [Credits](#credits) do the same work.
- **Failed sign-ins.** After 10 wrong passwords for one email within 15
  minutes, password sign-in for that email stops until the window ends,
  even with the right password. Existing sessions, tokens and API keys
  keep working, and `users set-password` always works.

## Credits

Credits are **off**: nobody is refused for running out, and the
per-account rate limit (60 calls a minute, `LIMITS_DEFAULT_RATE_PER_MIN`)
still applies.

To give each account a monthly allowance, set `CREDITS_ENABLED=true` in
`.env` and run `up -d` again.
- **The allowance:** every account gets a monthly free grant
  (`CREDITS_DEFAULT_MONTHLY_FREE_MC`, default 5,000 credits), and each
  accepted call costs `CREDITS_COST_PER_INVOCATION_MC` (default 0.1
  credit). Amounts are in milli-credits: 1 credit = 1,000.
- **Running out:** unused credit rolls over. An account that runs out gets
  429s until its next grant or a top-up.
- **Managing it:**

  ```sh
  docker compose -f docker-compose.prod.yml exec relay chakramcp-server credits show you@example.com
  ```

  `credits grant <account> <credits>` adds credit, `credits adjust` moves
  it either way (with a note), and `credits set` sets an account
  `--unlimited`, or its own `--monthly-grant` and `--rate`. `<account>` is
  an account slug or id, or a user's email for their personal account.
- **Turning credits on later:** accounts with no history start fresh,
  with their first grant. Accounts that already had credits keep their
  balance and receive the grants they missed, up to 12 months.

## Upgrades

```sh
git pull
docker compose -f docker-compose.prod.yml pull
docker compose -f docker-compose.prod.yml up -d
```

- `pull` is needed because `up -d` never re-pulls a tag it already has.
- If `git pull` changed the `Caddyfile`, run
  `docker compose -f docker-compose.prod.yml restart caddy`. `git pull`
  writes a new file, and Caddy's single-file mount keeps showing the old
  one.
- With observability, re-run its deploy step as well
  ([observability.md](observability.md#upgrades)).
- **Don't downgrade across a migration.** The server refuses to boot
  when the database has a migration it doesn't know. Roll forward
  instead.
- **From 0.2.0:**
  - Public sign-up closes, and `ADMIN_EMAIL` no longer makes anyone an
    admin. Existing admins keep the role; make new ones with
    `users set-admin`.
  - Credits turn off.
  - Set `SIGNUP_ENABLED` or `CREDITS_ENABLED` if you want either back.

**Pinning a version.** The default, `:latest`, moves with each release.
To stay on one, set `CHAKRAMCP_IMAGE` in `.env`, e.g.
`ghcr.io/delta-s-labs/chakramcp-server:0.3.0`. `:edge` follows every
change to `main`.

## Operating

- Logs: `docker compose -f docker-compose.prod.yml logs -f relay`. With
  the default journald driver, the host journal keeps them too
  (`journalctl CONTAINER_NAME=…`). The relay logs its mode at startup,
  e.g. `hosting_mode=self_hosted credits=off signup=closed`.
- A database shell:
  `docker compose -f docker-compose.prod.yml exec pg psql -U chakramcp`.
- A backup:
  `docker compose -f docker-compose.prod.yml exec -T pg pg_dump -U chakramcp chakramcp > backup.sql`.
- Settings live in `.env`. After changing one, run `up -d` again; Compose
  recreates only what changed.

## Without systemd (Docker Desktop)

Set `LOG_DRIVER=json-file` in `.env` and the services run anywhere
Docker does. json-file logs don't rotate by default, so set `max-size`
and `max-file` under `log-opts` in Docker's daemon settings. The
observability stack can't run on such a host.

For a local trial, the `*.localhost` names work without DNS. Use plain
HTTP, because the CLI only trusts public certificate authorities and so
wouldn't trust Caddy's local one:

```
APP_DOMAIN=http://app.localhost
RELAY_DOMAIN=http://relay.localhost
APP_PUBLIC_URL=http://app.localhost
RELAY_PUBLIC_URL=http://relay.localhost
```

The end-to-end test in [`infra/e2e/`](../../infra/e2e/run.sh) runs this
setup on every change.
