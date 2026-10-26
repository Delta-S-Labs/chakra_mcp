// Message safety in words: what was removed, flagged or blocked in a call,
// and why a message was refused.

import type { Safety, SafetyFinding } from "./relay";

const KINDS: Record<string, [string, string]> = {
  email: ["an email address", "email addresses"],
  phone: ["a phone number", "phone numbers"],
  url: ["a link", "links"],
  messaging_link: ["a messaging link", "messaging links"],
  handle: ["a handle", "handles"],
  off_platform: ["an attempt to move the conversation elsewhere", "attempts to move the conversation elsewhere"],
  opaque: ["unreadable content", "unreadable content"],
  unjudged: ["text too long to check in full", "text too long to check in full"],
  harm_violence: ["help with violence or weapons", "help with violence or weapons"],
  harm_crime: ["help with fraud or other crime", "help with fraud or other crime"],
  harm_harassment: ["harassment or threats", "harassment or threats"],
  harm_minors: ["sexual content involving minors", "sexual content involving minors"],
  harm_self: ["encouraging self-harm", "encouraging self-harm"],
  harm_malware: ["malware or breaking into systems", "malware or breaking into systems"],
  prompt_injection: ["instructions aimed at the other agent", "instructions aimed at the other agent"],
  off_capability: ["a task the capability doesn't offer", "tasks the capability doesn't offer"],
  off_purpose: ["something outside the grant's purpose", "things outside the grant's purpose"],
  off_relationship: ["something beyond what the agents agreed", "things beyond what the agents agreed"],
  data_exfiltration: ["a request for secrets or private data", "requests for secrets or private data"],
};

/** A finding's kind in words, counted: "a phone number", "2 phone numbers". */
export function kindLabel(kind: string, count = 1): string {
  const [one, many] = KINDS[kind] ?? [kind.replace(/_/g, " "), kind.replace(/_/g, " ")];
  if (count === 1) return one;
  return one === many ? many : `${count} ${many}`;
}

function counted(findings: SafetyFinding[]): string {
  const counts = new Map<string, number>();
  for (const f of findings) counts.set(f.kind, (counts.get(f.kind) ?? 0) + 1);
  return [...counts].map(([kind, n]) => kindLabel(kind, n)).join(", ");
}

/** One line per part and action, e.g. "Removed from the request: a phone
 *  number." Empty when there's nothing to say. */
export function describeSafety(safety: Safety | undefined | null): string[] {
  if (!safety) return [];
  return (["request", "reply"] as const).flatMap((part) =>
    describeFindings(safety[part], part),
  );
}

/** One line per action for one part's findings, e.g. a call message's:
 *  "Removed from the message: a phone number." */
export function describeFindings(
  findings: SafetyFinding[] | undefined | null,
  part: string,
): string[] {
  const lines: string[] = [];
  const of = (action: SafetyFinding["action"]) =>
    (findings ?? []).filter((f) => f.action === action);
  const blocked = of("blocked");
  const redacted = of("redacted");
  const flagged = of("flagged");
  if (blocked.length) lines.push(`Blocked the ${part}: ${counted(blocked)}.`);
  if (redacted.length) lines.push(`Removed from the ${part}: ${counted(redacted)}.`);
  if (flagged.length) lines.push(`Flagged in the ${part}: ${counted(flagged)}.`);
  return lines;
}

/** A blocked message's stored stand-in: the content isn't kept. */
export function isBlocked(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as Record<string, unknown>).__chakramcp_blocked__ === true
  );
}

/** What to tell someone whose message safety screening refused, or null
 *  when the error isn't a safety refusal. */
export function refusalMessage(err: unknown): string | null {
  const e = err as { code?: unknown; message?: unknown } | null;
  const code = typeof e?.code === "string" ? e.code : null;
  const message = typeof e?.message === "string" ? e.message : "";
  if (code === "chk.safety.too_large") {
    return "This message is too long to screen. Shorten it and try again.";
  }
  if (code !== "chk.policy.safety_blocked" && code !== "chk.policy.compliance_denied") {
    return null;
  }
  // "System One safety check blocked this note: harassment or threats
  // (harm_harassment=0.95)" → "harassment or threats".
  const why = message
    .replace(/^[^:]*:\s*/, "")
    .replace(/\s*\([a-z_]+=[0-9.]+\)/g, "")
    .trim();
  return why
    ? `Safety screening refused this message: ${why}. Change it and try again.`
    : "Safety screening refused this message. Change it and try again.";
}
