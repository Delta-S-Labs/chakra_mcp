"use server";

import { auth } from "@/auth";
import {
  ApiClientError,
  updateOrgSettings,
  type OrgSettings,
} from "@/lib/api";

export type FormResult =
  | { ok: true; settings: OrgSettings }
  | { ok: false; error: string };

export async function saveSettings(
  slug: string,
  body: Partial<OrgSettings>,
): Promise<FormResult> {
  const session = await auth();
  const token = session?.backendToken;
  if (!token) return { ok: false, error: "Not signed in." };

  try {
    const settings = await updateOrgSettings(token, slug, body);
    return { ok: true, settings };
  } catch (err) {
    if (err instanceof ApiClientError) {
      if (err.status === 403) {
        return { ok: false, error: "Only owners and admins can change settings." };
      }
      if (err.status === 400) {
        return { ok: false, error: err.message };
      }
    }
    return {
      ok: false,
      error: err instanceof Error ? err.message : "Save failed.",
    };
  }
}
