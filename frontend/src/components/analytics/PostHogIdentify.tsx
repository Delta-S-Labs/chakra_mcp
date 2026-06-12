"use client";

import { useEffect } from "react";

import { initPostHog, posthog } from "@/lib/posthog";

/**
 * Identifies the signed-in user to PostHog. Mounted only inside the
 * /app layout, so anonymous marketing-site visitors are never
 * identified — matching the `identified_only` person-profile mode.
 *
 * Calling `identify` with the same distinct id is idempotent, so it's
 * safe to render on every /app navigation. Sign-out calls
 * `posthog.reset()` from the UserMenu to detach the next visitor on a
 * shared browser.
 */
export function PostHogIdentify({
  userId,
  email,
  name,
}: {
  userId?: string;
  email?: string | null;
  name?: string | null;
}) {
  useEffect(() => {
    if (!process.env.NEXT_PUBLIC_POSTHOG_KEY || !userId) return;
    // Belt-and-braces: ensure init ran even if this child effect fires
    // before the provider's body init on first mount.
    initPostHog();
    posthog.identify(userId, {
      email: email ?? undefined,
      name: name ?? undefined,
    });
  }, [userId, email, name]);

  return null;
}
