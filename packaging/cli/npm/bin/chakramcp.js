#!/usr/bin/env node
// Thin shim that execs the native binary downloaded by install.js.
// Same directory as this script; .exe on Windows.

const path = require("path");
const { spawnSync } = require("child_process");

const binary = path.join(
  __dirname,
  process.platform === "win32" ? "chakramcp.exe" : "chakramcp",
);

const result = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: false,
});

if (result.error) {
  console.error(
    "[chakramcp] could not run the native binary at " + binary + ":",
    result.error.message,
  );
  console.error(
    "Try reinstalling: `npm i -g @chakramcp/cli` or grab a release from https://github.com/Delta-S-Labs/chakra_mcp/releases",
  );
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
