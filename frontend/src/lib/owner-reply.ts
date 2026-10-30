import type { ReplyShape } from "./relay";

/**
 * What the owner typed, as the call's output, by the reply shape the relay
 * gives the page (`shared::consent::owner_reply::ReplyShape`, the same
 * rules as the self-hosted page): the text itself, `message_owner`'s
 * `{status: "replied", reply, responded_at}`, or parsed JSON. A blank
 * reply, or JSON that doesn't parse, is a sentence to show instead.
 */
export function replyOutput(
  shape: ReplyShape,
  typed: string,
  now: Date = new Date(),
): { ok: true; output: unknown } | { ok: false; error: string } {
  const text = typed.trim();
  if (!text) return { ok: false, error: "Write a reply first." };
  switch (shape) {
    case "text":
      return { ok: true, output: text };
    case "replied":
      return {
        ok: true,
        output: {
          status: "replied",
          reply: text,
          responded_at: now.toISOString().replace(/\.\d{3}Z$/, "Z"),
        },
      };
    case "json":
      try {
        return { ok: true, output: JSON.parse(text) };
      } catch (err) {
        const why = err instanceof Error ? err.message : "it doesn't parse";
        return { ok: false, error: `The reply must be JSON for this capability: ${why}.` };
      }
  }
}
