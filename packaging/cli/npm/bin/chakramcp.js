#!/usr/bin/env node
// Thin shim that execs the native binary downloaded by install.js.
// Same directory as this script; .exe on Windows.
//
// npm 12 blocks install scripts by default, so the binary may not be here.
// Then it's fetched on first run: into this directory if it's writable,
// else into a per-user cache (~/.cache/chakramcp/<version>).

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const name = process.platform === "win32" ? "chakramcp.exe" : "chakramcp";

function cacheDir(version) {
  const base =
    process.env.XDG_CACHE_HOME ||
    (process.platform === "win32"
      ? process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local")
      : path.join(os.homedir(), ".cache"));
  return path.join(base, "chakramcp", version);
}

async function locate() {
  const here = path.join(__dirname, name);
  if (fs.existsSync(here)) return here;
  const { install, VERSION } = require("../install.js");
  const cached = path.join(cacheDir(VERSION), name);
  if (fs.existsSync(cached)) return cached;

  console.error("[chakramcp] fetching the native binary (first run)…");
  const log = (line) => console.error(line);
  let lastError;
  const writable = (dir) => {
    try {
      fs.accessSync(dir, fs.constants.W_OK);
      return true;
    } catch {
      return false;
    }
  };
  // The package's own directory when it's writable, else the user cache.
  const dirs = writable(__dirname) ? [__dirname, cacheDir(VERSION)] : [cacheDir(VERSION)];
  for (const dir of dirs) {
    try {
      const binary = await install(dir, log);
      if (fs.existsSync(binary)) return binary;
    } catch (err) {
      lastError = err;
    }
  }
  throw lastError || new Error("the download left no binary");
}

locate()
  .then((binary) => {
    const result = spawnSync(binary, process.argv.slice(2), {
      stdio: "inherit",
      windowsHide: false,
    });
    if (result.error) {
      console.error(
        "[chakramcp] could not run the native binary at " + binary + ":",
        result.error.message,
      );
      process.exit(1);
    }
    process.exit(result.status === null ? 1 : result.status);
  })
  .catch((err) => {
    console.error("[chakramcp] couldn't fetch the native binary:", err.message);
    console.error(
      "Install it another way: curl -fsSL https://chakramcp.com/install.sh | sh, or grab a release from https://github.com/Delta-S-Labs/chakra_mcp/releases",
    );
    process.exit(1);
  });
