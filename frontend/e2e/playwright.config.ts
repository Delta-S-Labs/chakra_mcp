/**
 * Playwright config for the local ChakraMCP smoke test.
 *
 * The spec talks to a manually-started local stack:
 *   - Postgres on :5432  (task db:up)
 *   - chakramcp-server on :8080 (app) + :8090 (relay)
 *     (cd backend && cargo run -p chakramcp-server -- start)
 *   - frontend dev server on :3000 (cd frontend && pnpm dev)
 *
 * Run with:
 *   pnpm e2e                 # headless
 *   pnpm e2e:ui              # debugger UI
 *
 * Screenshots and traces land under `e2e/screenshots/<run-id>/`. The
 * run-id is the test-run timestamp, set in a global setup hook so every
 * artefact a single `pnpm e2e` invocation produces shares the same dir.
 */
import { defineConfig } from "@playwright/test";
import path from "node:path";

// One timestamp per Playwright run — `Date.now()` here is evaluated when
// the config is first imported (process start), which happens once per
// `pnpm e2e` invocation, so every test in a run shares the same dir.
const RUN_ID =
  process.env.PLAYWRIGHT_RUN_ID ?? new Date().toISOString().replace(/[:.]/g, "-");

const SCREENSHOTS_DIR = path.join(__dirname, "screenshots", RUN_ID);

// Surface the run dir to specs so they can write phase-milestone PNGs.
process.env.PLAYWRIGHT_SCREENSHOTS_DIR = SCREENSHOTS_DIR;

export default defineConfig({
  testDir: __dirname,
  // The smoke spec is one ordered sequence (signup → pair → key →
  // invoke → dashboard → cleanup). Parallelism would break it, and a
  // retry on a half-cleaned-up failure would just create more orphans.
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  // Generous: the cold cargo build (first run after `cargo clean`) takes
  // a while, and inbox draining + invoke polling has its own latencies.
  // Per-test cap is the slow end of "everything works".
  timeout: 180_000,
  expect: { timeout: 15_000 },
  reporter: [
    ["list"],
    ["html", { outputFolder: path.join(SCREENSHOTS_DIR, "html-report"), open: "never" }],
  ],
  outputDir: path.join(SCREENSHOTS_DIR, "test-results"),
  use: {
    baseURL: process.env.E2E_FRONTEND_URL ?? "http://localhost:3000",
    // `screenshot: 'on'` writes a PNG at the end of every test; we ALSO
    // call `page.screenshot()` at every phase milestone for the
    // "evidence at each step" requirement.
    screenshot: "on",
    video: "retain-on-failure",
    trace: "retain-on-failure",
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
  },
  projects: [
    {
      name: "chromium",
      use: {
        // Use the bundled Chromium — no need for a system browser. The
        // first run will download via `pnpm exec playwright install`.
        browserName: "chromium",
        viewport: { width: 1280, height: 800 },
      },
    },
  ],
});
