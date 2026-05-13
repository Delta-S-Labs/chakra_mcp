// Renders at the top of /login + /signup when the user already has
// an active session cookie. Without this, the middleware would
// silently bounce them to /app — which felt like the auth pages were
// "auto-logging-in" their last account. Now the user gets an explicit
// choice: continue as the signed-in user, or sign out and pick again.

import Link from "next/link";
import { redirect } from "next/navigation";
import { revalidatePath } from "next/cache";
import { auth, signOut } from "@/auth";
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
        <form
          action={async () => {
            "use server";
            // NextAuth v5 beta races the NEXT_REDIRECT throw against
            // the Set-Cookie writes when you pass `redirectTo` here —
            // the cookie sometimes survives sign-out, leaving the
            // banner stuck on /login + /signup. The fix is to do the
            // signOut + redirect in two explicit steps:
            //   1. signOut({ redirect: false }) — synchronously writes
            //      the cookie-clearing Set-Cookie headers and returns.
            //   2. revalidatePath("/", "layout") — invalidates any RSC
            //      cache that still has the old session memo'd in.
            //   3. redirect("/login") — normal Next redirect throw.
            await signOut({ redirect: false });
            revalidatePath("/", "layout");
            redirect("/login");
          }}
        >
          <button type="submit" className={styles.alreadyInSignOut}>
            Sign out and switch
          </button>
        </form>
      </div>
    </div>
  );
}
