"use server";

import { revalidatePath } from "next/cache";
import { auth } from "@/auth";
import { approveRequest, denyRequest, RelayClientError, revokeConsent } from "@/lib/relay";

export type DecisionResult = { ok: true } | { ok: false; error: string };

/** What a failed decision says to the approver. */
function failure(err: unknown, what: string): DecisionResult {
  if (err instanceof RelayClientError) {
    switch (err.status) {
      case 400:
      case 409:
        return { ok: false, error: err.message };
      case 401:
        return { ok: false, error: "Your session ended. Sign in again to decide." };
      case 403:
        return { ok: false, error: "Only this capability's approvers can decide on it." };
      case 404:
        return { ok: false, error: "This request no longer exists." };
    }
  }
  return { ok: false, error: `Couldn't ${what}. Try again.` };
}

async function token(): Promise<string | null> {
  const session = await auth();
  return session?.backendToken ?? null;
}

/**
 * Approve a held request, as asked or narrowed. The relay checks that the
 * caller is an approver, signed in to the web app, and that the terms stay
 * within the request and the publisher's limit.
 */
export async function approveAction(
  id: string,
  validFor: string,
  maxRequests: number | null,
): Promise<DecisionResult> {
  const t = await token();
  if (!t) return { ok: false, error: "Your session ended. Sign in again to decide." };
  try {
    await approveRequest(t, id, {
      valid_for: validFor,
      ...(maxRequests ? { max_requests: maxRequests } : {}),
    });
  } catch (err) {
    return failure(err, "approve it");
  }
  revalidatePath("/app/approvals");
  return { ok: true };
}

/** Deny a held request; the requester sees the note. */
export async function denyAction(id: string, note: string): Promise<DecisionResult> {
  const t = await token();
  if (!t) return { ok: false, error: "Your session ended. Sign in again to decide." };
  try {
    await denyRequest(t, id, note.trim() || undefined);
  } catch (err) {
    return failure(err, "deny it");
  }
  revalidatePath("/app/approvals");
  return { ok: true };
}

/** Revoke a standing consent: it stops covering new requests at once. */
export async function revokeAction(consentId: string): Promise<DecisionResult> {
  const t = await token();
  if (!t) return { ok: false, error: "Your session ended. Sign in again." };
  try {
    await revokeConsent(t, consentId);
  } catch (err) {
    return failure(err, "revoke it");
  }
  revalidatePath("/app/approvals");
  return { ok: true };
}
