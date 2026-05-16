/**
 * Shared range helpers for the /app/usage page. Split out of
 * `page.tsx` so the client-side `UsageView` can import them without
 * dragging the server-only `export const metadata = ...` through the
 * bundle. Next 16 rejects `metadata` exports from any module that the
 * client graph touches.
 */

export type RangeKey = "7d" | "30d" | "90d";

export function computeRange(key: RangeKey): { from: string; to: string } {
  const to = new Date();
  const days = key === "7d" ? 7 : key === "90d" ? 90 : 30;
  const from = new Date(to.getTime() - days * 86_400_000);
  return { from: from.toISOString(), to: to.toISOString() };
}

/**
 * Window key from `?range=` — falls back to "30d" so URL-less landings
 * hit the default. Anything unrecognised also collapses to "30d" so we
 * never render a blank chart.
 */
export function readRangeParam(
  params: Record<string, string | string[] | undefined>,
): RangeKey {
  const v = params.range;
  const raw = Array.isArray(v) ? v[0] : v;
  if (raw === "7d" || raw === "30d" || raw === "90d") return raw;
  return "30d";
}
