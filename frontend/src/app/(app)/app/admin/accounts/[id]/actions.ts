"use server";

import { auth } from "@/auth";
import {
  adminAddCreditEntry,
  adminUpdateCreditSettings,
  ApiClientError,
  type CreditEntryRequest,
  type CreditSettingsRequest,
} from "@/lib/api";

export type ActionResult = { ok: true } | { ok: false; error: string };

export async function addCreditEntry(
  accountId: string,
  body: CreditEntryRequest,
): Promise<ActionResult> {
  return asAdmin((token) => adminAddCreditEntry(token, accountId, body));
}

export async function updateCreditSettings(
  accountId: string,
  body: CreditSettingsRequest,
): Promise<ActionResult> {
  return asAdmin((token) => adminUpdateCreditSettings(token, accountId, body));
}

async function asAdmin(run: (token: string) => Promise<unknown>): Promise<ActionResult> {
  const session = await auth();
  const token = session?.backendToken;
  if (!token || !session?.user?.is_admin) return { ok: false, error: "Admins only." };
  try {
    await run(token);
    return { ok: true };
  } catch (err) {
    if (err instanceof ApiClientError) {
      if (err.status === 403) return { ok: false, error: "Admins only." };
      if (err.status === 400 || err.status === 404) return { ok: false, error: err.message };
    }
    return { ok: false, error: err instanceof Error ? err.message : "Request failed." };
  }
}
