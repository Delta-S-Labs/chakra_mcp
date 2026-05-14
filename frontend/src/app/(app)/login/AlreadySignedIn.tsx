// Renders at the top of /login + /signup when the user already has
// an active session cookie. Without this, the middleware would
// silently bounce them to /app — which felt like the auth pages were
// "auto-logging-in" their last account. Now the user gets an explicit
// choice: continue as the signed-in user, or sign out and pick again.
//
// The sign-out form delegates to `signOutAndRedirect` (see
// `src/lib/auth-actions.ts`) which does the belt-and-braces cookie
// purge needed under NextAuth v5 beta + Next.js 16.

import Link from "next/link";
import { auth } from "@/auth";
import { signOutAndRedirect } from "@/lib/auth-actions";
import styles from "./login.module.css";

export async function AlreadySignedIn() {
  const session = await auth();
  if (!session?.user) return null;

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
        <Link href="/app" className={styles.alreadyInContinue}>
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
