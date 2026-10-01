/**
 * Unit tests (`pnpm test`): plain functions under src/, named *.test.ts.
 * They run on Playwright's test runner, which the e2e suite already
 * brings, and never start a browser.
 */
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "src",
  testMatch: "**/*.test.ts",
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? "dot" : "list",
});
