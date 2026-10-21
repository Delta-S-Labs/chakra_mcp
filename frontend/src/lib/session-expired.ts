import { redirect } from "next/navigation";

/**
 * Expired sessions on /app pages.
 *
 * A NextAuth cookie can outlive the backend token it carries: the token
 * expires, or signing out elsewhere revokes it. The app shell still renders
 * (it only checks that a session exists), and the first API call then fails
 * with 401. Pages send the reader back to sign in rather than show the
 * backend's bare "unauthorized"; /login shows a "session ended" note for
 * `reason=session_expired` and returns to `from` afterwards.
 */

/** True when an API call failed because the session's backend token was refused. */
export function isUnauthorized(err: unknown): boolean {
  if (!(err instanceof Error)) return false;
  // ApiClientError (app) and RelayClientError (relay) both carry the status.
  const status = (err as { status?: unknown }).status;
  if (typeof status === "number") return status === 401;
  return /\b401\b|unauthori[sz]ed/i.test(err.message);
}

/** Where to send someone whose session has ended: sign in, then back to `from`. */
export function sessionExpiredUrl(from: string): string {
  return `/login?reason=session_expired&from=${encodeURIComponent(from)}`;
}

/**
 * On a 401, ends the request with a redirect to sign in and back to `from`
 * (the page's own path). Otherwise returns, so the caller handles the error.
 *
 * Call it inside a `catch`, never inside the `try`: `redirect()` works by
 * throwing, and a surrounding `try` would swallow it.
 */
export function redirectIfUnauthorized(err: unknown, from: string): void {
  if (isUnauthorized(err)) redirect(sessionExpiredUrl(from));
}
