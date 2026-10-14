import type { CreditAlert } from "./api";

/** One line of the app's credits banner. */
export interface AlertLine {
  key: string;
  tone: "low" | "blocked";
  text: string;
  href: string;
}

export interface AlertBanner {
  lines: AlertLine[];
  /** "and 2 more", when some alerts don't fit. */
  more: string | null;
  /** "Buy credits" where credits can be bought, else "View credits". */
  action: string;
  /** Only a banner of low balances can be dismissed; out-of-credits stays. */
  dismissible: boolean;
  /** Changes whenever the set of alerts does, so a dismissal ends then. */
  dismissKey: string;
}

/** At most this many accounts get their own line. */
export const MAX_LINES = 3;

/**
 * What the banner says for the user's accounts that are running low or out
 * of credits, or `null` when none are. Pure, so it's unit-tested.
 */
export function creditBanner(alerts: CreditAlert[], purchaseAvailable: boolean): AlertBanner | null {
  if (alerts.length === 0) return null;
  // Out of credits first: it's what stops calls.
  const ordered = [...alerts].sort(
    (a, b) => Number(b.state === "blocked") - Number(a.state === "blocked"),
  );
  const lines = ordered.slice(0, MAX_LINES).map((a) => ({
    key: a.account_slug,
    tone: a.state,
    text: lineText(a, purchaseAvailable),
    href: `/app/credits?account=${encodeURIComponent(a.account_slug)}`,
  }));
  const hidden = ordered.length - lines.length;
  return {
    lines,
    more: hidden > 0 ? `and ${hidden} more` : null,
    action: purchaseAvailable ? "Buy credits" : "View credits",
    dismissible: alerts.every((a) => a.state === "low"),
    dismissKey: ordered
      .map((a) => `${a.account_slug}:${a.state}`)
      .sort()
      .join(","),
  };
}

function lineText(a: CreditAlert, purchaseAvailable: boolean): string {
  const name = a.account_type === "individual" ? "Your personal account" : a.account_name;
  if (a.state === "blocked") {
    const until = formatDay(a.next_grant_on);
    return purchaseAvailable
      ? `${name} is out of credits. Calls are refused until ${until}, or until you buy more.`
      : `${name} is out of credits. Calls are refused until ${until}.`;
  }
  const calls = a.calls_left.toLocaleString("en-US");
  return `${name} is low on credits: about ${calls} ${a.calls_left === 1 ? "call" : "calls"} left.`;
}

function formatDay(isoDate: string): string {
  return new Date(`${isoDate}T00:00:00Z`).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  });
}
