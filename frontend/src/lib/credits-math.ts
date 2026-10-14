/**
 * Money and credits for the buy box. Amounts stay integers end to end:
 * cents for dollars, milli-credits for credits (1 credit = 1,000 mc). Only
 * formatting divides.
 */

/**
 * What someone typed as a dollar amount, in cents: "10", "10.5", ".50",
 * "$1,000.25". Anything else (negative, more than two decimals, junk,
 * empty) is `null`.
 */
export function parseDollars(input: string): number | null {
  let s = input.trim().replace(/^\$\s*/, "").replace(/,/g, "");
  if (s.startsWith(".")) s = `0${s}`;
  const match = /^(\d+)(?:\.(\d{1,2}))?$/.exec(s);
  if (!match) return null;
  const cents = Number(match[1]) * 100 + Number((match[2] ?? "").padEnd(2, "0"));
  return Number.isSafeInteger(cents) ? cents : null;
}

/** Milli-credits that `cents` buys at `creditsPerUsd`, as the server computes them. */
export function creditsMcFor(cents: number, creditsPerUsd: number): number {
  return cents * creditsPerUsd * 10;
}

/** How many calls `mc` pays for at `costMc` each. */
export function callsFor(mc: number, costMc: number): number {
  return costMc > 0 ? Math.max(0, Math.floor(mc / costMc)) : 0;
}

const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });

/** `237` → "$2.37". */
export function formatUsd(cents: number): string {
  return usd.format(cents / 100);
}
