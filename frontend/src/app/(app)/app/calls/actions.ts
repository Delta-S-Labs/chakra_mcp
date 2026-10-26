"use server";

import { revalidatePath } from "next/cache";
import { auth } from "@/auth";
import { deleteCallContent, ownerEndCall, RelayClientError } from "@/lib/relay";

export type CallActionResult = { ok: true } | { ok: false; error: string };

function failure(err: unknown, what: string): CallActionResult {
  if (err instanceof RelayClientError) {
    switch (err.status) {
      case 401:
        return { ok: false, error: "Your session ended. Sign in again." };
      case 403:
        return { ok: false, error: "Only an owner of one of this call's agents can do that." };
      case 404:
        return { ok: false, error: "This call no longer exists." };
      case 409:
        return { ok: false, error: "This call has already ended." };
    }
  }
  return { ok: false, error: `Couldn't ${what}. Try again.` };
}

async function token(): Promise<string | null> {
  const session = await auth();
  return session?.backendToken ?? null;
}

/** End a ringing or active call. Both agents are told. */
export async function endCallAction(id: string): Promise<CallActionResult> {
  const t = await token();
  if (!t) return { ok: false, error: "Your session ended. Sign in again." };
  try {
    await ownerEndCall(t, id);
  } catch (err) {
    return failure(err, "end the call");
  }
  revalidatePath(`/app/calls/${id}`);
  revalidatePath("/app/calls");
  return { ok: true };
}

/** Delete a call's content now, for both sides; an open call is ended. */
export async function deleteContentAction(id: string): Promise<CallActionResult> {
  const t = await token();
  if (!t) return { ok: false, error: "Your session ended. Sign in again." };
  try {
    await deleteCallContent(t, id);
  } catch (err) {
    return failure(err, "delete the content");
  }
  revalidatePath(`/app/calls/${id}`);
  revalidatePath("/app/calls");
  return { ok: true };
}
