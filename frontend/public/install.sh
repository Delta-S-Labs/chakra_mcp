#!/bin/sh
# chakramcp install script — fetches the latest CLI for this OS/arch
# from GitHub Releases and drops the binary into a directory on $PATH.
#
# Usage:
#   curl -fsSL https://chakramcp.com/install.sh | sh
#
# Env knobs:
#   INSTALL_DIR — defaults to /usr/local/bin (or ~/.local/bin if not writable)
#   VERSION     — pin to a specific release (e.g. VERSION=0.1.0). Default: latest.

set -eu

REPO="${CHAKRAMCP_REPO:-Delta-S-Labs/chakra_mcp}"
DEFAULT_DIR="/usr/local/bin"
[ -w "$DEFAULT_DIR" ] || DEFAULT_DIR="$HOME/.local/bin"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_DIR}"

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux)  os="unknown-linux-gnu" ;;
  *) echo "chakramcp installer: unsupported OS '$(uname -s)' — try the manual download from https://github.com/${REPO}/releases" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64)   arch="x86_64" ;;
  arm64|aarch64)  arch="aarch64" ;;
  *) echo "chakramcp installer: unsupported arch '$(uname -m)'" >&2; exit 1 ;;
esac

target="${arch}-${os}"

if [ -z "${VERSION:-}" ]; then
  api="https://api.github.com/repos/${REPO}/releases?per_page=20"
  tag="$(curl -fsSL "$api" | tr ',' '\n' | grep '"tag_name"' | grep '"cli-v' | head -n 1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
  if [ -z "$tag" ]; then
    echo "chakramcp installer: couldn't find a cli-v* release on ${REPO}" >&2
    exit 1
  fi
else
  tag="cli-v${VERSION}"
fi
ver="${tag#cli-v}"

archive="chakramcp-${ver}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${tag}/${archive}"

echo "==> Installing chakramcp ${ver} (${target}) to ${INSTALL_DIR}"
mkdir -p "$INSTALL_DIR"

tmp="$(mktemp -d -t chakramcp.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$url" -o "$tmp/cli.tar.gz" || {
  echo "Download failed: $url" >&2
  exit 1
}

# Verify checksum if available; non-fatal on miss so the install
# still works even if a hosting hop strips the .sha256 sidecar.
sum_url="${url}.sha256"
if curl -fsSL "$sum_url" -o "$tmp/cli.sha256" 2>/dev/null; then
  expected="$(awk 'NR==1 {print $1}' "$tmp/cli.sha256")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/cli.tar.gz" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$tmp/cli.tar.gz" | awk '{print $1}')"
  fi
  if [ -n "${actual:-}" ] && [ "$actual" != "$expected" ]; then
    echo "chakramcp installer: sha256 mismatch — refusing to install" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi
fi

tar -xzf "$tmp/cli.tar.gz" -C "$tmp"
install -m 0755 "$tmp/chakramcp" "$INSTALL_DIR/chakramcp"

echo "==> Installed to $INSTALL_DIR/chakramcp"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Note: $INSTALL_DIR isn't on \$PATH — add it or move the binary." >&2 ;;
esac
"$INSTALL_DIR/chakramcp" --version
