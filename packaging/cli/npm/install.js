#!/usr/bin/env node
// Postinstall — fetches the matching native chakramcp binary from the
// matching GitHub Release and drops it next to bin/chakramcp.js so the
// JS shim can exec it.
//
// The version comes from this package's own version field (synced in
// the release workflow). If you need to override (e.g. installing from
// a fork), set CHAKRAMCP_REPO and CHAKRAMCP_VERSION env vars.

const fs = require("fs");
const os = require("os");
const path = require("path");
const https = require("https");
const { execSync } = require("child_process");

const REPO = process.env.CHAKRAMCP_REPO || "Delta-S-Labs/chakra_mcp";
const VERSION = process.env.CHAKRAMCP_VERSION || require("./package.json").version;

function targetFor(platform, arch) {
  const map = {
    "darwin-arm64": "aarch64-apple-darwin",
    "darwin-x64":   "x86_64-apple-darwin",
    "linux-arm64":  "aarch64-unknown-linux-gnu",
    "linux-x64":    "x86_64-unknown-linux-gnu",
    "win32-x64":    "x86_64-pc-windows-msvc",
  };
  const key = `${platform}-${arch}`;
  if (!(key in map)) {
    throw new Error(`unsupported platform/arch: ${key}`);
  }
  return map[key];
}

function fetchBuffer(url, redirects = 5) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { "user-agent": "chakramcp-npm-install" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location && redirects > 0) {
        resolve(fetchBuffer(res.headers.location, redirects - 1));
        return;
      }
      if (res.statusCode !== 200) {
        reject(new Error(`HTTP ${res.statusCode} fetching ${url}`));
        return;
      }
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    }).on("error", reject);
  });
}

async function main() {
  const target = targetFor(process.platform, process.arch);
  const isWindows = process.platform === "win32";
  const archive = isWindows
    ? `chakramcp-${VERSION}-${target}.zip`
    : `chakramcp-${VERSION}-${target}.tar.gz`;
  const url = `https://github.com/${REPO}/releases/download/cli-v${VERSION}/${archive}`;

  console.log(`[chakramcp] downloading ${archive}`);
  const buf = await fetchBuffer(url);

  const binDir = path.join(__dirname, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const tmpArchive = path.join(os.tmpdir(), `chakramcp-${process.pid}-${archive}`);
  fs.writeFileSync(tmpArchive, buf);

  if (isWindows) {
    // Use built-in PowerShell Expand-Archive.
    execSync(
      `powershell -Command "Expand-Archive -Path '${tmpArchive}' -DestinationPath '${binDir}' -Force"`,
      { stdio: "inherit" },
    );
    // Resulting file: bin/chakramcp.exe
  } else {
    execSync(`tar -xzf '${tmpArchive}' -C '${binDir}'`, { stdio: "inherit" });
    fs.chmodSync(path.join(binDir, "chakramcp"), 0o755);
  }
  fs.unlinkSync(tmpArchive);
  console.log(`[chakramcp] installed ${target} ${VERSION}`);
}

main().catch((err) => {
  console.error("[chakramcp] install failed:", err.message);
  console.error(
    "Fall back to: curl -fsSL https://chakramcp.com/install.sh | sh",
  );
  process.exit(1);
});
