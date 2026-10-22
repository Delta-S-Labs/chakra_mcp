import { cache } from "react";
import { auth } from "@/auth";
import { listApprovals } from "@/lib/relay";

/**
 * How many requests wait for the signed-in user's approval: once per
 * render, shared by the nav badge and the banner. Null when it can't be
 * read (no session, relay down); callers then show nothing.
 */
export const waitingApprovals = cache(async (): Promise<number | null> => {
  const session = await auth();
  const token = session?.backendToken;
  if (!token) return null;
  try {
    return (await listApprovals(token, "waiting")).length;
  } catch {
    return null;
  }
});
