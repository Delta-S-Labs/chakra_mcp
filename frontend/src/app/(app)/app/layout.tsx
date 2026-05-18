import type { ReactNode } from "react";
import Link from "next/link";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { AppNav } from "./AppNav";
import { BottomTabBar } from "./BottomTabBar";
import { CommandPalette } from "./CommandPalette";
import { UserMenu } from "./UserMenu";
import styles from "./shell.module.css";

/**
 * Layout for `/app` and its children.
 *
 * Renders the app shell - top bar with brandmark, main nav, and a
 * profile dropdown (UserMenu) that holds API keys / Pair agent /
 * Audit / Admin plus sign out. Children render in <main>.
 *
 * The `src/proxy.ts` middleware also gates this route group, but we
 * defensively re-check in case (a) middleware misses an edge case
 * (e.g. RSC sub-fetches that don't pass through the proxy) or (b)
 * the NextAuth cookie survives signOut() in one of the Netlify-edge
 * + NextAuth v5 beta failure modes documented in auth-actions.ts.
 * If we land here without a real session, redirect — never render a
 * stub that looks like a half-broken app shell.
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
    redirect("/login?from=%2Fapp");
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
      <BottomTabBar />
      <CommandPalette />
    </div>
  );
}
