import type { PaymentStatus } from "./api";

/**
 * What Dodo's `status` on the return URL says happened: `succeeded`,
 * `failed`, `processing`, `expired`, …. Anyone can edit a URL, so this only
 * picks what the page says while it waits. Whether credits were added is
 * always the backend's status.
 */
export type ReturnHint = "succeeded" | "not_paid" | "unknown";

export function returnHint(status: unknown): ReturnHint {
  if (status === "succeeded") return "succeeded";
  if (status === "failed" || status === "cancelled" || status === "expired") return "not_paid";
  // `processing`, a `requires_*` step, or nothing at all.
  return "unknown";
}

/** How long to wait for the backend to agree with Dodo's "it didn't go through". */
export const NOT_PAID_WAIT_MS = 10_000;
/** How long to wait for the webhook otherwise. */
export const CONFIRM_WAIT_MS = 120_000;

export type PollVerdict = "keep" | "done" | "review" | "declined" | "pending";

/**
 * After a look at the backend: keep polling, or what to tell the buyer.
 * `status` is the latest status the backend gave (`null` before the first
 * answer).
 */
export function pollVerdict(
  hint: ReturnHint,
  status: PaymentStatus | null,
  waitedMs: number,
): PollVerdict {
  if (status === "paid") return "done";
  if (status === "unapplied") return "review";
  const notPaid = status === "failed" || status === "expired";
  if (hint === "not_paid" && (notPaid || waitedMs >= NOT_PAID_WAIT_MS)) return "declined";
  // After a success, a `failed` can be an earlier declined attempt in the
  // same checkout, with the successful one's webhook still on its way.
  if (waitedMs >= CONFIRM_WAIT_MS) return notPaid ? "declined" : "pending";
  return "keep";
}
