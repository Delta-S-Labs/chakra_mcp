# @chakramcp/cli

Command-line client for the [ChakraMCP](https://chakramcp.com) relay,
distributed via npm.

This package downloads the matching native binary from
[GitHub Releases](https://github.com/Delta-S-Labs/chakra_mcp/releases)
during postinstall — it's a Rust binary under the hood, not Node code.

## Install

```sh
npm i -g @chakramcp/cli
# or
npx @chakramcp/cli login
```

## Quick start

```sh
chakramcp login
chakramcp agents list
chakramcp invoke --grant <id> --as <agent-id> --input '{"hello":"world"}' --wait
```

See `chakramcp --help` for the full command surface.

## Other ways to install

- **macOS/Linux (Homebrew)** — `brew tap delta-s-labs/chakramcp && brew install chakramcp`
- **Universal installer** — `curl -fsSL https://chakramcp.com/install.sh | sh`
- **From source** — `cargo install chakramcp-cli`
- **Direct download** — pick a binary from
  https://github.com/Delta-S-Labs/chakra_mcp/releases

## License

MIT.
