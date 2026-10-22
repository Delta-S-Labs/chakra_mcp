/**
 * Owner consent helpers for the approval pages: what an approver may choose,
 * and how terms read. The relay checks every decision again; these keep the
 * pages from offering anything it would refuse. Mirrors
 * `chakramcp_shared::consent::terms` (`narrowing_options`, `describe`).
 */

import type { ConsentTerms } from "./relay";

const HOUR = 3600;
const DAY = 24 * HOUR;

/** `once`, `<n>h` or `<n>d` in seconds (0 for once); null if malformed. */
export function validitySeconds(raw: string | undefined | null): number | null {
  const v = (raw ?? "once").trim();
  if (v === "once") return 0;
  const m = /^(\d+)([hd])$/.exec(v);
  if (!m) return null;
  const n = Number(m[1]);
  if (n < 1) return null;
  return m[2] === "d" ? n * DAY : n * HOUR;
}

/** Seconds as `once`, `<n>d` or `<n>h`. */
export function validityString(secs: number): string {
  if (secs <= 0) return "once";
  return secs % DAY === 0 ? `${secs / DAY}d` : `${secs / HOUR}h`;
}

/** "1 hour", "36 hours", "1 day", "7 days". */
function windowLabel(secs: number): string {
  const [n, unit] = secs % DAY === 0 ? [secs / DAY, "day"] : [secs / HOUR, "hour"];
  return n === 1 ? `1 ${unit}` : `${n} ${unit}s`;
}

/** A validity choice in words: "Just this request", "7 days". */
export function validityLabel(raw: string): string {
  const secs = validitySeconds(raw) ?? 0;
  return secs === 0 ? "Just this request" : windowLabel(secs);
}

/** Terms in words: "Just this request", "For 7 days, up to 5 requests". */
export function describeTerms(terms: ConsentTerms | null | undefined): string {
  const secs = validitySeconds(terms?.valid_for) ?? 0;
  if (secs === 0) return "Just this request";
  const window = windowLabel(secs);
  const max = terms?.max_requests;
  if (max === 1) return `For ${window}, 1 request`;
  if (max) return `For ${window}, up to ${max} requests`;
  return `For ${window}`;
}

/**
 * The validities an approver may choose, longest first: the request itself
 * (capped at the publisher's limit), the standard shorter windows, then
 * "once". Never longer than asked or allowed.
 */
export function narrowingOptions(requested: string | undefined, maxValidity: string): string[] {
  const asked = validitySeconds(requested) ?? 0;
  const max = validitySeconds(maxValidity) ?? 0;
  const ceiling = Math.min(asked, max);
  const options: string[] = [];
  if (ceiling > 0) {
    options.push(validityString(ceiling));
    for (const secs of [30 * DAY, 7 * DAY, DAY, HOUR]) {
      if (secs < ceiling) options.push(validityString(secs));
    }
  }
  options.push("once");
  return options;
}

/** "3 hours left", "12 minutes left", "Expired". */
export function timeLeft(expiresAt: string, now: Date = new Date()): string {
  const ms = new Date(expiresAt).getTime() - now.getTime();
  if (!(ms > 0)) return "Expired";
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${Math.max(minutes, 1)} minute${minutes === 1 ? "" : "s"} left`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} hour${hours === 1 ? "" : "s"} left`;
  const days = Math.floor(hours / 24);
  return `${days} days left`;
}

/** The marker the relay shows the target side instead of a held input. */
export function isHeldInput(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as Record<string, unknown>).__chakramcp_held__ === true
  );
}

/** A held input's top-level fields as text, for the approval page. */
export function inputFields(input: unknown): { name: string; value: string }[] {
  if (input === null || input === undefined) return [];
  const text = (v: unknown) => (typeof v === "string" ? v : JSON.stringify(v));
  if (typeof input === "object" && !Array.isArray(input)) {
    return Object.entries(input as Record<string, unknown>).map(([name, value]) => ({
      name,
      value: text(value),
    }));
  }
  return [{ name: "input", value: text(input) }];
}
