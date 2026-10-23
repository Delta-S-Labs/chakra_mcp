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
        return { ok: false, error: capitalize(err.message) };
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

function capitalize(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? "" : ".") : s;
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

/**
 * Confirm a reply's draft: it must be the one the approver was shown
 * (`draftSha256`), or the relay answers that it changed.
 */
export async function confirmReplyAction(id: string, draftSha256: string): Promise<DecisionResult> {
  const t = await token();
  if (!t) return { ok: false, error: "Your session ended. Sign in again to decide." };
  try {
    await approveRequest(t, id, { draft_sha256: draftSha256 });
  } catch (err) {
    return failure(err, "confirm it");
  }
  revalidatePath("/app/approvals");
  return { ok: true };
}

/** Deny a held request (or reject a reply); the requester sees the note. */
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
