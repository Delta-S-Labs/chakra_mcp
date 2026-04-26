# Installing the chakramcp CLI

The CLI is a single Rust binary. Pick whichever channel fits your stack.

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

Required org-level config (only the npm one is needed; the formula
lives in this same repo and uses the auto-provided GITHUB_TOKEN):
- Repo secret `NPM_TOKEN` — npm publish token for `@chakramcp/cli`

The Homebrew job opens a PR (`release-bot/homebrew-<version>` branch
→ `main`) on every tagged release. Merge the PR (or set up auto-merge)
to publish the new formula. The npm job is conditional on `NPM_TOKEN`,
so the very first release runs fine without it set.
