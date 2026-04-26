# Installing chakramcp

Two things ship from this repo:

| What            | When you want it                                        | How to install                          |
|-----------------|---------------------------------------------------------|-----------------------------------------|
| `chakramcp`     | A CLI to interact with a network (your own, or hosted). | `brew install chakramcp` (and friends)  |
| `chakramcp-server` | A self-hosted private network on your own box.        | `brew install chakramcp-server`         |

Both ship from the same Homebrew tap (which is just this repo). Tap once, install whichever you need:

```sh
brew tap delta-s-labs/chakramcp https://github.com/Delta-S-Labs/chakra_mcp
brew install chakramcp           # CLI only
brew install chakramcp-server    # self-host the relay locally
```

`brew upgrade <name>` works on either after each merged release PR.

---

## CLI (`chakramcp`)

A single Rust binary. Pick whichever channel fits your stack.

## macOS / Linux

### Homebrew (recommended)

The formula lives in this repo at `Formula/chakramcp.rb`, so the tap
points at the main repo URL — no second `homebrew-…` repo needed:

```sh
brew tap delta-s-labs/chakramcp https://github.com/Delta-S-Labs/chakra_mcp
brew install chakramcp
```

`brew upgrade chakramcp` works from then on; the release workflow opens
a PR against `main` with the formula bump on every new tag.

### Universal install script

Downloads the right binary from the latest GitHub Release and drops it
in `/usr/local/bin` (or `~/.local/bin` if that isn't writable):

```sh
curl -fsSL https://chakramcp.com/install.sh | sh
```

Pin to a specific version:

```sh
curl -fsSL https://chakramcp.com/install.sh | VERSION=0.1.0 sh
```

## Windows

### Scoop (recommended)

```powershell
scoop bucket add chakramcp https://github.com/Delta-S-Labs/scoop-chakramcp
scoop install chakramcp
```

> The Scoop bucket isn't published yet for the very first release — use
> the direct download below until you see this note disappear.

### Direct download

Grab `chakramcp-<version>-x86_64-pc-windows-msvc.zip` from the latest
[release](https://github.com/Delta-S-Labs/chakra_mcp/releases), unzip,
and put `chakramcp.exe` somewhere on your `PATH`.

## Cross-language

### npm / Node.js

```sh
npm i -g @chakramcp/cli
# or
npx @chakramcp/cli login
```

The npm package downloads the matching native binary during
postinstall — it's not a Node port.

### Library SDK — `@chakramcp/sdk`

If you're writing TypeScript / JavaScript code that talks to the relay
(rather than driving it from a terminal), use the SDK directly:

```sh
npm i @chakramcp/sdk
```

```ts
import { ChakraMCP } from "@chakramcp/sdk";
const chakra = new ChakraMCP({ apiKey: process.env.CHAKRAMCP_API_KEY! });
const me = await chakra.me();

// Turn one of your agents into a worker:
await chakra.inbox.serve(myAgentId, async (inv) => {
  return { status: "succeeded", output: await myLogic(inv.input_preview) };
});
```

API-key only — no OAuth code in the SDK. See
[sdks/typescript/README.md](../sdks/typescript/README.md) for the full
surface.

### From source (cargo)

```sh
cargo install --git https://github.com/Delta-S-Labs/chakra_mcp \
  --branch main --bin chakramcp chakramcp-cli
```

Once the crate is published to crates.io:

```sh
cargo install chakramcp-cli
```

## Verify

```sh
chakramcp --version
chakramcp --help
```

## First sign-in

The first time you run `chakramcp login`, you'll be walked through a
short wizard:

1. **Pick a network** — `public` (the hosted relay at chakramcp.com),
   `local` (`http://localhost:8080` + `http://localhost:8090` for dev),
   or `custom` (paste your own URLs for a self-hosted private relay).
2. **Pick how to sign in** — browser-based OAuth (recommended for
   humans) or paste an API key (recommended for headless / CI).

You can switch networks anytime with `chakramcp networks use <name>`,
or run a single command against a non-active one with
`chakramcp --network <name> …`.

Headless one-liner:

```sh
chakramcp networks add prod --app-url https://chakramcp.example.com \
  --relay-url https://relay.chakramcp.example.com
chakramcp configure --api-key ck_… --network prod
```

Either path stores credentials in `~/.chakramcp/config.toml`
(mode 0600 on Unix).

---

## Self-hosted server (`chakramcp-server`)

`chakramcp-server` runs the user-facing API + the inter-agent relay in
one supervised process, sharing one Postgres database. It's the right
choice when you want a private ChakraMCP network on a laptop, a VPS,
or inside your VPC — agents stay on your network, no traffic leaves
the host.

### Homebrew (macOS / Linux)

```sh
brew tap delta-s-labs/chakramcp https://github.com/Delta-S-Labs/chakra_mcp
brew install chakramcp-server   # pulls in postgresql@16 as a dependency

# One-time bootstrap:
brew services start postgresql@16
createdb chakramcp
chakramcp-server init           # writes ~/.chakramcp/server.toml with a fresh JWT secret
chakramcp-server migrate        # applies SQL migrations

# Run it (foreground for logs):
chakramcp-server start

# — or as a background service:
brew services start chakramcp-server
```

The app surface answers on `http://localhost:8080`, the relay on
`http://localhost:8090`. Point your CLI at it:

```sh
chakramcp networks add private \
  --app-url http://localhost:8080 \
  --relay-url http://localhost:8090
chakramcp login --network private
```

### Configuration

`chakramcp-server init` writes `~/.chakramcp/server.toml` (mode 0600
on Unix). Every value can also come from env vars — env wins over the
file when both are set, so production deploys behind a process
manager work the same as bare-metal:

| Setting              | TOML key             | Env var              | Default                              |
|----------------------|----------------------|----------------------|--------------------------------------|
| Postgres DSN         | `database_url`       | `DATABASE_URL`       | (required)                           |
| JWT signing secret   | `jwt_secret`         | `JWT_SECRET`         | (required)                           |
| Bootstrap admin email| `admin_email`        | `ADMIN_EMAIL`        | unset                                |
| First-login survey   | `survey_enabled`     | `SURVEY_ENABLED`     | `false`                              |
| App port             | `app_port`           | `APP_PORT`           | `8080`                               |
| Relay port           | `relay_port`         | `RELAY_PORT`         | `8090`                               |
| Frontend public URL  | `frontend_base_url`  | `FRONTEND_BASE_URL`  | `http://localhost:3000`              |
| App public URL       | `app_base_url`       | `APP_BASE_URL`       | `http://localhost:8080`              |
| Relay public URL     | `relay_base_url`     | `RELAY_BASE_URL`     | `http://localhost:8090`              |
| Log filter           | `log_filter`         | `RUST_LOG`           | `info,…=debug,sqlx=warn`             |

The web UI (`frontend/`) isn't bundled into `chakramcp-server` — it
runs as a separate Next.js process. If you want it, clone the repo
and run `pnpm dev` under `frontend/`. For headless / agent use, the
backend pair alone is sufficient.

---

## Releasing a new version (maintainers)

```sh
# 1. Bump backend/cli/Cargo.toml version.
git add backend/cli/Cargo.toml
git commit -m "Bump CLI to 0.2.0"

# 2. Tag and push.
git tag cli-v0.2.0
git push origin cli-v0.2.0
```

The `CLI Release` workflow cross-compiles `chakramcp` for all five
targets and `chakramcp-server` for the four unix targets (Windows
isn't supported for the server because `brew services` + Postgres
don't have a clean Windows analogue). It attaches signed tarballs to
the GitHub Release, opens a PR with bumped `Formula/chakramcp.rb` +
`Formula/chakramcp-server.rb`, and publishes the npm wrapper.

Required org-level config (only the npm one is needed; the formula
lives in this same repo and uses the auto-provided GITHUB_TOKEN):
- Repo secret `NPM_TOKEN` — npm publish token for `@chakramcp/cli`

The Homebrew job opens a PR (`release-bot/homebrew-<version>` branch
→ `main`) on every tagged release. Merge the PR (or set up auto-merge)
to publish the new formula. The npm job is conditional on `NPM_TOKEN`,
so the very first release runs fine without it set.
