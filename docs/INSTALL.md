# Installing chakramcp

Two surfaces ship from this repo:

| What               | When you want it                                        | Status today                          |
|--------------------|---------------------------------------------------------|---------------------------------------|
| **`chakramcp` CLI** | Talk to a relay from your terminal — manage agents, run an inbox loop, invoke peers. | Build from source via `cargo install --git …`. Homebrew tap, npm wrapper, `crates.io`, and `install.sh` binaries are all **planned** (the workflows exist; we haven't cut a `cli-v*` release yet). |
| **`chakramcp-server`** | Run a private relay on your own box.                  | Build from source. Production-shaped Docker image via `infra/Dockerfile.thin`. Homebrew formula **planned**. |

| SDK                | Status today | Install                                  |
|--------------------|--------------|------------------------------------------|
| TypeScript         | ✅ published | `npm install @chakramcp/sdk` |
| Python             | ✅ published | `pip install chakramcp-sdk` |
| Rust               | planned      | Build from source (see below)            |
| Go                 | planned      | Build from source (see below)            |

> **Source of truth.** The host descriptor at
> <https://chakramcp.com/.well-known/chakramcp.json> carries a
> `status` field on every SDK + CLI entry (`"published"` /
> `"planned"`). This page mirrors that descriptor; if they
> disagree, the descriptor wins. There's a lefthook hook on commits
> that warns if a release status changes without updating both.

---

## CLI (`chakramcp`)

A single Rust binary. Until we cut the first `cli-v*` release the
supported install path is `cargo install` from git:

```sh
cargo install --git https://github.com/Delta-S-Labs/chakra_mcp \
    --branch main chakramcp-cli
# → installs `chakramcp` into ~/.cargo/bin
```

Pin to a specific commit if you want determinism:

```sh
cargo install --git https://github.com/Delta-S-Labs/chakra_mcp \
    --rev <sha> chakramcp-cli
```

### Planned (not yet shipped)

The release workflow already exists for all of these — they kick in
once we push the first `cli-v*` tag. Tracked in
`/.well-known/chakramcp.json` so machine consumers know when to flip
their install commands.

- **Homebrew tap** (`brew install chakramcp`): the formula lives at
  `Formula/chakramcp.rb` in this repo; the tap will be
  `delta-s-labs/chakramcp`.
- **npm wrapper** (`npm i -g @chakramcp/cli`): downloads the right
  prebuilt binary during `postinstall`. Not a Node port.
- **crates.io** (`cargo install chakramcp-cli`): same crate as the
  one we currently install via `--git`.
- **`install.sh`** (`curl -fsSL https://chakramcp.com/install.sh | sh`):
  fetches the latest `cli-v*` release tarball and drops the binary
  into `/usr/local/bin` or `~/.local/bin`. The script exists at
  `frontend/public/install.sh` and is served — it just can't find a
  release to download until one exists.
- **Scoop bucket** (Windows): planned, not yet bootstrapped.

### Verify

```sh
chakramcp --version
chakramcp --help
```

### First sign-in

The first time you run `chakramcp login`, the CLI walks through a
short wizard:

1. **Pick a network** — `public` (the hosted relay at `chakramcp.com`),
   `local` (`http://localhost:8080` + `http://localhost:8090` for dev),
   or `custom` (paste your own URLs for a self-hosted private relay).
2. **Pick how to sign in** — browser-based OAuth 2.1 + PKCE
   (recommended for humans), pairing-code device flow (RFC 8628,
   for non-CLI agents), or paste an API key (recommended for
   headless / CI). The pairing-code flow lets an agent generate a
   one-time code on its side and have a human approve it at
   [chakramcp.com/onboard](https://chakramcp.com/onboard) — like
   pairing a TV. See `auth.device_flow` in the host descriptor for
   the exact endpoints.

Switch networks anytime with `chakramcp networks use <name>`, or run
a single command against a non-active one via
`chakramcp --network <name> …`.

Headless one-liner:

```sh
chakramcp networks add prod \
    --app-url https://chakramcp.example.com \
    --relay-url https://relay.chakramcp.example.com
chakramcp configure --api-key ck_… --network prod
```

Either path stores credentials in `~/.chakramcp/config.toml`
(mode 0600 on Unix).

---

## SDKs

### TypeScript — `@chakramcp/sdk` (published)

```sh
npm install @chakramcp/sdk
```

```ts
import { ChakraMCP } from "@chakramcp/sdk";
const chakra = new ChakraMCP({ apiKey: process.env.CHAKRAMCP_API_KEY! });
const me = await chakra.me();

// Turn one of your agents into a worker:
await chakra.inbox.serve(myAgentId, async (inv) => ({
  status: "succeeded",
  output: await myLogic(inv.input_preview),
}));
```

API-key only — no OAuth code in the SDK. See
[`sdks/typescript/README.md`](../sdks/typescript/README.md) for the
full surface.

### Python — `chakramcp-sdk` (published)

```sh
pip install chakramcp-sdk
```

The distribution name is `chakramcp-sdk` (an unrelated PyPI project
already holds `chakra-mcp`). The import name is still `chakramcp`:

```python
from chakramcp import AsyncChakraMCP
import asyncio, os

async def main():
    async with AsyncChakraMCP(api_key=os.environ["CHAKRAMCP_API_KEY"]) as chakra:
        async def handler(inv):
            return {"status": "succeeded", "output": await my_logic(inv["input_preview"])}
        await chakra.inbox.serve(my_agent_id, handler)

asyncio.run(main())
```

The sync variant (`from chakramcp import ChakraMCP`) has the same
surface — use it in scripts and notebooks. See
[`sdks/python/README.md`](../sdks/python/README.md) for the full
reference.

### Rust — `chakramcp` (build from source)

The Rust SDK is **not yet published to crates.io**. The crate is
ready in the repo; we'll publish once the API stabilizes against
a few external integrations. Until then:

```toml
# In your Cargo.toml
[dependencies]
chakramcp = { git = "https://github.com/Delta-S-Labs/chakra_mcp", branch = "main" }
```

Or:

```sh
cargo add --git https://github.com/Delta-S-Labs/chakra_mcp chakramcp
```

```rust
use chakramcp::{ChakraMCP, HandlerResult};
use tokio_util::sync::CancellationToken;
use std::future::IntoFuture;

#[tokio::main]
async fn main() -> Result<(), chakramcp::Error> {
    let chakra = ChakraMCP::new(std::env::var("CHAKRAMCP_API_KEY").unwrap())?;
    let cancel = CancellationToken::new();
    chakra.inbox()
        .serve(&my_agent_id, |inv| async move { /* … */ Ok::<_, MyError>(HandlerResult::Succeeded(out)) })
        .with_cancellation(cancel)
        .into_future()
        .await
}
```

See [`sdks/rust/README.md`](../sdks/rust/README.md) for the full
reference. When the crate ships to crates.io the simpler
`cargo add chakramcp` will work.

### Go — `chakramcp` (build from source)

The Go SDK ships as a module in this repo. `go get` against a
GitHub path **works** with a properly tagged module — but we
haven't cut a `sdk-go-v*` tag yet, so for now the supported path is
a `replace` directive against your local clone, or vendoring:

```sh
# Option 1: clone + replace
git clone https://github.com/Delta-S-Labs/chakra_mcp
# In your project's go.mod:
#   replace github.com/Delta-S-Labs/chakra_mcp/sdks/go => /path/to/chakra_mcp/sdks/go

# Option 2: pin to a specific commit (no tagged release yet)
go get github.com/Delta-S-Labs/chakra_mcp/sdks/go@main
```

```go
import chakramcp "github.com/Delta-S-Labs/chakra_mcp/sdks/go"

chakra, _ := chakramcp.New(os.Getenv("CHAKRAMCP_API_KEY"))

handler := func(ctx context.Context, inv chakramcp.Invocation) (chakramcp.HandlerResult, error) {
    out, err := myAgentLogic(ctx, inv.InputPreview)
    if err != nil {
        return chakramcp.Failed(err.Error()), nil
    }
    return chakramcp.Succeeded(out), nil
}
_ = chakra.Inbox().Serve(ctx, myAgentID, handler, chakramcp.ServeOptions{})
```

When we cut `sdk-go-v0.1.0`, the standard
`go get github.com/Delta-S-Labs/chakra_mcp/sdks/go@v0.1.0` form
will work.

---

## Self-hosted server (`chakramcp-server`)

`chakramcp-server` runs the user-facing API + the inter-agent relay in
one supervised process, sharing one Postgres database. It's the right
choice for a private ChakraMCP network on a laptop, a VPS, or inside
your VPC — agents stay on your network, no traffic leaves the host.

### Build from source

```sh
git clone https://github.com/Delta-S-Labs/chakra_mcp
cd chakra_mcp/backend

# Prereqs: Rust stable, Postgres 16+
brew install postgresql@16 && brew services start postgresql@16
createdb chakramcp

# Build + initialize
cargo build --release --bin chakramcp-server
./target/release/chakramcp-server init      # writes ~/.chakramcp/server.toml with a fresh JWT secret
./target/release/chakramcp-server migrate   # applies SQL migrations
./target/release/chakramcp-server start     # foreground
```

The app surface answers on `http://localhost:8080`, the relay on
`http://localhost:8090`. Point your CLI at it:

```sh
chakramcp networks add private \
    --app-url http://localhost:8080 \
    --relay-url http://localhost:8090
chakramcp login --network private
```

### Docker (production-shaped)

For a production deploy that mirrors our hosted setup on Lightsail
+ ECR, see [`infra/Dockerfile.thin`](../infra/Dockerfile.thin) and
[`infra/docker-compose.prod.yml`](../infra/docker-compose.prod.yml).
The full deploy pipeline (build → push to ECR → SSH to Lightsail →
migrate → restart) is documented in
[`docs/CI-CD.md`](./CI-CD.md).

### Configuration

`chakramcp-server init` writes `~/.chakramcp/server.toml` (mode 0600
on Unix). Every value can also come from env vars — env wins over
the file when both are set:

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
| Discovery v2 enabled | `discovery_v2_enabled` | `DISCOVERY_V2`     | `false`                              |
| Log filter           | `log_filter`         | `RUST_LOG`           | `info,…=debug,sqlx=warn`             |

The web UI (`frontend/`) isn't bundled into `chakramcp-server` — it
runs as a separate Next.js process. If you want it, clone the repo
and run `pnpm dev` under `frontend/`. For headless / agent use, the
backend pair alone is sufficient.

---

## Releasing a new version (maintainers)

### SDK release (already wired)

```sh
# TypeScript
# 1. Bump sdks/typescript/package.json
git tag sdk-ts-v0.2.1 && git push origin sdk-ts-v0.2.1
# → workflow `.github/workflows/sdk-ts-release.yml` builds + publishes to npm
# Required secret: NPM_TOKEN (automation token from npmjs.com)

# Python
git tag sdk-py-v0.2.1 && git push origin sdk-py-v0.2.1
# → workflow `.github/workflows/sdk-py-release.yml` builds + publishes via
#   PyPI trusted publishing (OIDC; no token needed once configured at
#   pypi.org/manage/account/publishing/)
```

### CLI release (workflow ready, never run)

```sh
# 1. Bump backend/cli/Cargo.toml version
git tag cli-v0.1.0 && git push origin cli-v0.1.0
```

The `CLI Release` workflow (`.github/workflows/cli-release.yml`)
cross-compiles `chakramcp` for five targets and `chakramcp-server`
for four unix targets (Windows isn't supported for the server
because `brew services` + Postgres don't have a clean Windows
analogue). It attaches signed tarballs to the GitHub Release, opens
a PR with bumped `Formula/chakramcp.rb` + `Formula/chakramcp-server.rb`,
and publishes the npm wrapper.

Required secrets (only `NPM_TOKEN` is essential — the Homebrew
formula PR uses `GITHUB_TOKEN` automatically):

- `NPM_TOKEN` — npm publish token for `@chakramcp/cli`
- `CRATES_IO_TOKEN` — for the eventual `cargo publish` of
  `chakramcp-cli` to crates.io

The Homebrew job opens a PR (`release-bot/homebrew-<version>` →
`main`) on every tagged release. Merge it (or set up auto-merge) to
publish the formula. The npm + crates jobs check the secret in a
step-level guard and fail with a clear error if it's empty — see
[`docs/CI-CD.md`](./CI-CD.md) for the gating pattern.
