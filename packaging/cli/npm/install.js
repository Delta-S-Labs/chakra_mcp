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
  pathHint();
}

// After a global install, npm links the `chakramcp` bin into the global
// prefix's bin dir. If that dir isn't on the user's PATH, the command is
// "not found" even though the install succeeded — the single most common
// confusion. Detect it and print the exact line to fix it. Best-effort:
// never throws, never blocks the install.
function globalBinDir() {
  // npm exposes the resolved prefix as an env var during lifecycle
  // scripts — prefer it (no subprocess, works even when `npm` isn't on
  // the script shell's PATH). Fall back to asking npm directly.
  let prefix = process.env.npm_config_prefix;
  if (!prefix) {
    try {
      prefix = execSync("npm config get prefix", { encoding: "utf8" }).trim();
    } catch {
      return null;
    }
  }
  if (!prefix) return null;
  // Windows drops shims directly in <prefix>; unix in <prefix>/bin.
  return process.platform === "win32" ? prefix : path.join(prefix, "bin");
}

function pathHint() {
  try {
    // Only relevant for global installs — local installs are invoked via
    // npx / package.json scripts, not a bare `chakramcp`.
    if (process.env.npm_config_global !== "true") return;
    const binDir = globalBinDir();
    if (!binDir) return;
    const onPath = (process.env.PATH || "")
      .split(path.delimiter)
      .some((p) => {
        try {
          return p && path.resolve(p) === path.resolve(binDir);
        } catch {
          return false;
        }
      });
    if (onPath) return;

    console.log("");
    console.log("[chakramcp] ⚠  `chakramcp` was installed but its directory is");
    console.log("            not on your PATH, so the command won't be found yet:");
    console.log(`              ${binDir}`);
    console.log("");
    console.log("[chakramcp] add it so it's callable from anywhere:");
    if (process.platform === "win32") {
      console.log(`              setx PATH "${binDir};%PATH%"`);
      console.log("            then open a new terminal.");
    } else {
      const rc = (process.env.SHELL || "").includes("bash")
        ? "~/.bashrc"
        : "~/.zshrc";
      console.log(`              echo 'export PATH="${binDir}:$PATH"' >> ${rc}`);
      console.log(`              source ${rc}`);
    }
    console.log("");
    console.log("[chakramcp] then verify:  chakramcp --version");
    console.log("");
  } catch {
    /* a missing PATH hint must never fail the install */
  }
}

main().catch((err) => {
  console.error("[chakramcp] install failed:", err.message);
  console.error(
    "Fall back to: curl -fsSL https://chakramcp.com/install.sh | sh",
  );
  process.exit(1);
});
