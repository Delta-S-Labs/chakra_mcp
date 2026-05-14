import type { ReactNode } from "react";
import Link from "next/link";
import { auth } from "@/auth";
import { AppNav } from "./AppNav";
import { UserMenu } from "./UserMenu";
import styles from "./shell.module.css";

/**
 * Layout for `/app` and its children.
 *
 * Renders the app shell - top bar with brandmark, main nav, and a
 * profile dropdown (UserMenu) that holds API keys / Pair agent /
 * Audit / Admin plus sign out. Children render in <main>.
 *
 * Middleware ensures we never reach here without a session, but we
 * still defensively check.
 *
 * Sign-out lives inside UserMenu and delegates to
 * `signOutAndRedirect` (see `src/lib/auth-actions.ts`) — that helper
 * does the belt-and-braces cookie purge needed under NextAuth v5 beta
 * + Next.js 16 (raw `signOut()` leaves cookies behind in some edge
 * paths).
 */
export default async function AppShellLayout({ children }: { children: ReactNode }) {
  const session = await auth();
  if (!session?.user) {
    return (
      <main className={styles.page}>
        <div className={styles.shell}>
          <p>Not signed in.</p>
        </div>
      </main>
    );
  }

  const { name, email, image, is_admin } = session.user;

  return (
    <div className={styles.page}>
      <div className={styles.shell}>
        <header className={styles.header}>
          <Link href="/" className={styles.brandmark} aria-label="ChakraMCP home">
            <span className={styles.dot} aria-hidden="true" />
            <span className={styles.brandWord}>ChakraMCP</span>
          </Link>

          <AppNav />

          <UserMenu
            name={name}
            email={email}
            image={image}
            isAdmin={!!is_admin}
          />
        </header>

        <main className={styles.main}>{children}</main>
      </div>
    </div>
  );
}
