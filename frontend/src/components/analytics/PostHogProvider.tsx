"use client";

import { Suspense, useEffect } from "react";
import { usePathname, useSearchParams } from "next/navigation";
import { PostHogProvider as PHProvider } from "posthog-js/react";

import { initPostHog, posthog } from "@/lib/posthog";

/**
 * Wraps the whole app (mounted in the root layout). Initializes
 * PostHog on the client and captures a $pageview on every App Router
 * navigation.
 *
 * Init runs in the component body (guarded + idempotent) rather than
 * an effect so the singleton is ready before any deeper child effect
 * — notably <PostHogIdentify> in the /app layout — tries to use it.
 * `initPostHog` is a no-op on the server and when no key is set.
 */
export function PostHogProvider({ children }: { children: React.ReactNode }) {
  initPostHog();

  return (
    <PHProvider client={posthog}>
      <Suspense fallback={null}>
        <PostHogPageView />
      </Suspense>
      {children}
    </PHProvider>
  );
}

/**
 * Manual pageview capture for the App Router. Fires on pathname or
 * query-string change. Lives under <Suspense> because
 * `useSearchParams()` opts the subtree into client-side rendering and
 * Next.js requires a Suspense boundary around it.
 */
function PostHogPageView() {
  const pathname = usePathname();
  const searchParams = useSearchParams();

  useEffect(() => {
    if (!pathname || !process.env.NEXT_PUBLIC_POSTHOG_KEY) return;
    let url = window.location.origin + pathname;
    const qs = searchParams?.toString();
    if (qs) url += `?${qs}`;
    posthog.capture("$pageview", { $current_url: url });
  }, [pathname, searchParams]);

  return null;
}
