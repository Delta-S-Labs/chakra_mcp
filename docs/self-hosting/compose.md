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
   - `ADMIN_EMAIL`: this account gets the server's admin role.

   Everything else is optional and documented in the file.

4. **Start.**

   ```sh
   docker compose -f docker-compose.prod.yml up -d
   docker compose -f docker-compose.prod.yml ps
   ```

   The relay applies database migrations as it boots. Caddy requests the
   certificates within a minute or so. Check the result with
   `curl https://relay.example.com/healthz`.

5. **Connect a client.** Point the CLI at your network:

   ```sh
   chakramcp networks add private \
       --app-url https://app.example.com \
       --relay-url https://relay.example.com
   chakramcp login --network private
   ```

   MCP clients use `https://relay.example.com/mcp`.

To add dashboards and alerts, continue with
[observability.md](observability.md).

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

**Pinning a version.** The default, `:latest`, moves with each release.
To stay on one, set `CHAKRAMCP_IMAGE` in `.env`, e.g.
`ghcr.io/delta-s-labs/chakramcp-server:0.2.0`. `:edge` follows every
change to `main`.

## Operating

- Logs: `docker compose -f docker-compose.prod.yml logs -f relay`. With
  the default journald driver, the host journal keeps them too
  (`journalctl CONTAINER_NAME=…`).
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

For a local trial, the `*.localhost` names work without DNS:
`APP_DOMAIN=app.localhost`, `RELAY_DOMAIN=relay.localhost`, and the
matching `https://` URLs. Caddy then uses certificates from its own local
authority, which your machine doesn't trust: expect a browser warning,
and use `curl -k`.
