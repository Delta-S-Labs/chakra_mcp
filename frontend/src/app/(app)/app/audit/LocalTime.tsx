"use client";

import { useSyncExternalStore } from "react";

/**
 * Render a UTC ISO timestamp in the viewer's LOCAL timezone.
 *
 * The audit page is a server component and the server (Netlify) runs in
 * UTC, so formatting there shows UTC. `useSyncExternalStore` lets us
 * render the raw ISO on the server / first paint (no hydration mismatch)
 * and the browser-local string on the client — without a setState-in-
 * effect.
 */
const noopSubscribe = () => () => {};

export function LocalTime({ iso }: { iso: string }) {
  const isClient = useSyncExternalStore(
    noopSubscribe,
    () => true, // client snapshot
    () => false, // server snapshot
  );
  let text = iso;
  if (isClient) {
    try {
      text = new Date(iso).toLocaleString();
    } catch {
      /* keep raw iso */
    }
  }
  return <span suppressHydrationWarning>{text}</span>;
}
