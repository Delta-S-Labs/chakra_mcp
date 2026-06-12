import posthog from "posthog-js";

/**
 * Idempotent, client-only PostHog initializer.
 *
 * Called from both the provider (so anonymous pageviews start
 * immediately) and the in-app identify component (belt-and-braces, so
 * `identify()` never races ahead of `init()` — child effects fire
 * before parent effects on mount, and the identify component lives
 * deeper in the tree than the provider).
 *
 * No-ops when:
 *   - rendering on the server (`window` undefined)
 *   - already initialized this session
 *   - `NEXT_PUBLIC_POSTHOG_KEY` is unset → analytics is simply off, so
 *     local dev / self-hosters who don't want tracking get nothing
 *     loaded, no errors.
 *
 * Ingestion is reverse-proxied through `/ingest` (see next.config.ts
 * rewrites) so events are first-party and survive ad-blockers. Person
 * profiles are `identified_only`: anonymous marketing visitors never
 * get a profile; we only create one when a signed-in /app user is
 * explicitly identified.
 */
let initialized = false;

export function initPostHog(): void {
  if (typeof window === "undefined" || initialized) return;
  const key = process.env.NEXT_PUBLIC_POSTHOG_KEY;
  if (!key) return;

  posthog.init(key, {
    api_host: "/ingest",
    ui_host: "https://us.posthog.com",
    // App Router has no built-in route-change event; we capture
    // $pageview manually in PostHogProvider, so disable the automatic
    // first-load one to avoid double counting.
    capture_pageview: false,
    capture_pageleave: true,
    person_profiles: "identified_only",
  });
  initialized = true;
}

export { posthog };
