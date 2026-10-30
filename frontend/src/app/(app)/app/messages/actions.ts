"use server";

import { revalidatePath } from "next/cache";
import { auth } from "@/auth";
import { RelayClientError, reportResult } from "@/lib/relay";
import { refusalMessage } from "@/lib/safety";

export type AnswerResult = { ok: true } | { ok: false; error: string };

function capitalize(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? "" : ".") : s;
}

/**
 * Answer a call to a `human_in_loop` capability as one of its approvers:
 * the relay releases it at once, whether or not the agent claimed it, and
 * screens it first when message safety is on. `output` is what the reply box
 * became (`replyOutput`); a decline sends `failed` with the reason.
 */
export async function answerAction(
  id: string,
  answer: { status: "succeeded"; output: unknown } | { status: "failed"; error: string },
): Promise<AnswerResult> {
  const session = await auth();
  const token = session?.backendToken ?? null;
  if (!token) return { ok: false, error: "Your session ended. Sign in again to answer." };
  try {
    await reportResult(token, id, answer);
  } catch (err) {
    const refused = refusalMessage(err);
    if (refused) return { ok: false, error: refused };
    if (err instanceof RelayClientError) {
      switch (err.status) {
        case 400:
        case 409:
        case 413:
          return { ok: false, error: capitalize(err.message) };
        case 401:
          return { ok: false, error: "Your session ended. Sign in again to answer." };
        case 403:
        case 404:
          return { ok: false, error: "Only this agent's approvers can answer it." };
      }
    }
    return { ok: false, error: "Couldn't send your answer. Try again." };
  }
  revalidatePath("/app/approvals");
  revalidatePath(`/app/messages/${id}`);
  return { ok: true };
}
