// Renders at the top of /login + /signup ONLY when the user has a
// session that can actually reach /app. Without this, the middleware
// would silently bounce a valid-session user to /app — which felt like
// the auth pages were "auto-logging-in" their last account. The banner
// gives them an explicit choice: continue, or sign out and switch.
//
// Critical subtlety: the NextAuth cookie (which `auth()` reads) lives
// far longer than the *backend* access token embedded inside it — and
// that token is never refreshed (see auth.ts jwt callback). So the
// cookie routinely outlives the token. If we showed the banner on a
// cookie-alive-but-token-dead session, "Continue" would just loop the
// user /app → 401 → /login?reason=session_expired → back here. So we
// gate the banner on the embedded token still being usable; when it
// isn't, we render nothing and the user gets a clean login form.
//
// The sign-out form delegates to `signOutAndRedirect` (see
// `src/lib/auth-actions.ts`) which does the belt-and-braces cookie
// purge needed under NextAuth v5 beta + Next.js 16.

import Link from "next/link";
import { auth } from "@/auth";
import { signOutAndRedirect } from "@/lib/auth-actions";
import styles from "./login.module.css";

/**
 * Local, signature-free check that the embedded backend JWT hasn't
 * expired. We're not authenticating with it here — the real check
 * happens server-side at /app — we only need to decide whether
 * offering "Continue" would actually work. Decode the payload, read
 * `exp` (seconds), and keep a small skew buffer. Any malformed/missing
 * token counts as unusable.
 */
function backendTokenUsable(token: string | undefined): boolean {
  if (!token) return false;
  try {
    const payload = token.split(".")[1];
    if (!payload) return false;
    const claims = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
    return typeof claims.exp === "number" && claims.exp * 1000 > Date.now() + 5_000;
  } catch {
    return false;
  }
}

export async function AlreadySignedIn({ from }: { from?: string }) {
  const session = await auth();
  if (!session?.user) return null;

  // Cookie alive but the backend token it carries is gone/expired →
  // the session can't reach /app. Don't offer a dead "Continue"; let
  // the login form render instead.
  if (!backendTokenUsable(session.backendToken)) return null;

  const { name, email } = session.user;
  const label = name || email || "signed in";

  return (
    <div className={styles.alreadyIn}>
      <div className={styles.alreadyInText}>
        <strong>Signed in as {label}</strong>
        {email && name && (
          <span className={styles.alreadyInEmail}> · {email}</span>
        )}
      </div>
      <div className={styles.alreadyInActions}>
        <Link href={from || "/app"} className={styles.alreadyInContinue}>
          Continue
        </Link>
        <form action={signOutAndRedirect}>
          <button type="submit" className={styles.alreadyInSignOut}>
            Sign out and switch
          </button>
        </form>
      </div>
    </div>
  );
}
