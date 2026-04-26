# Installing the chakramcp CLI

The CLI is a single Rust binary. Pick whichever channel fits your stack.

## macOS / Linux

### Homebrew (recommended)

```sh
brew tap delta-s-labs/chakramcp
brew install chakramcp
```

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

Two paths — pick whichever fits:

- **Interactive (humans)** — `chakramcp login` opens a browser, you
  sign in to ChakraMCP, the CLI captures the OAuth callback on a
  loopback port.
- **Headless (CI, agents)** — generate an API key from
  `chakramcp.com/app/api-keys`, then `chakramcp configure --api-key
  ck_…`. Either path stores credentials in `~/.chakramcp/config.toml`
  (mode 0600 on Unix).

## Releasing a new version (maintainers)

```sh
# 1. Bump backend/cli/Cargo.toml version.
git add backend/cli/Cargo.toml
git commit -m "Bump CLI to 0.2.0"

# 2. Tag and push.
git tag cli-v0.2.0
git push origin cli-v0.2.0
```

The `CLI Release` workflow cross-compiles for all five targets,
attaches signed tarballs to the GitHub Release, bumps the Homebrew
formula in the tap repo, and publishes the npm wrapper.

Required org-level config:
- Repo variable `HOMEBREW_TAP_REPO` (e.g. `Delta-S-Labs/homebrew-chakramcp`)
- Repo secret `HOMEBREW_TAP_TOKEN` — PAT with `contents:write` on the tap repo
- Repo secret `NPM_TOKEN` — npm publish token for `@chakramcp/cli`

Both side-effect jobs are conditional, so the very first release runs
fine without them set.
