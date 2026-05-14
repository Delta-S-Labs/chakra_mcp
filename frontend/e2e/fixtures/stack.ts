/**
 * Health checks for the three services the smoke test depends on. Run
 * once at the top of the spec — failing fast here is much friendlier
 * than waiting 30s for a Playwright navigation to time out against an
 * unreachable :3000.
 */

import { APP_BASE_URL, RELAY_BASE_URL } from "./api";

const FRONTEND_BASE_URL =
  process.env.E2E_FRONTEND_URL ?? "http://localhost:3000";

async function ping(url: string, timeoutMs = 1_500): Promise<number | null> {
  const ctl = new AbortController();
  const id = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetch(url, { signal: ctl.signal });
    return res.status;
  } catch {
    return null;
  } finally {
    clearTimeout(id);
  }
}

async function waitFor(
  label: string,
  url: string,
  isHealthy: (status: number) => boolean,
  budgetMs = 15_000,
): Promise<void> {
  const deadline = Date.now() + budgetMs;
  let last: number | null = null;
  while (Date.now() < deadline) {
    last = await ping(url);
    if (last !== null && isHealthy(last)) return;
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(
    `${label} unreachable at ${url} (last status: ${last ?? "no response"}). ` +
      `Did you start the local stack? See frontend/e2e/README.md for startup order.`,
  );
}

/** chakramcp-app exposes /healthz (returns 200 when ready). */
export function waitForBackend(budgetMs = 15_000) {
  return waitFor("backend", `${APP_BASE_URL}/healthz`, (s) => s === 200, budgetMs);
}

/** chakramcp-relay shares the same /healthz convention. */
export function waitForRelay(budgetMs = 15_000) {
  return waitFor("relay", `${RELAY_BASE_URL}/healthz`, (s) => s === 200, budgetMs);
}

/**
 * The frontend root may 200 or 307-redirect (e.g. to the marketing site
 * route group). Any sub-500 response means Next.js is serving — good
 * enough for "the dev server is up".
 */
export function waitForFrontend(budgetMs = 30_000) {
  return waitFor(
    "frontend",
    `${FRONTEND_BASE_URL}/login`,
    (s) => s >= 200 && s < 500,
    budgetMs,
  );
}

export async function waitForStack() {
  // Run in parallel — they're independent and we want the failure
  // message to point at whichever piece is actually missing, not the
  // first-checked one.
  await Promise.all([waitForBackend(), waitForRelay(), waitForFrontend()]);
}

export { FRONTEND_BASE_URL };
