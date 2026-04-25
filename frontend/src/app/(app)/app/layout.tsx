import type { ReactNode } from "react";
import Link from "next/link";
import { auth, signOut } from "@/auth";
import { AppNav } from "./AppNav";
import styles from "./shell.module.css";

/**
 * Layout for `/app` and its children.
 *
 * Renders the app shell — top bar with brandmark, user menu, sign-out
 * button. Children render in <main>.
 *
 * Middleware ensures we never reach here without a session, but we
 * still defensively check.
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

          <AppNav isAdmin={!!is_admin} />

          <div className={styles.user}>
            {image && (
              // eslint-disable-next-line @next/next/no-img-element
              <img src={image} alt="" className={styles.avatar} />
            )}
            <div className={styles.userMeta}>
              <div className={styles.userName}>{name ?? "Signed in"}</div>
              {email && <div className={styles.userEmail}>{email}</div>}
            </div>
            <form
              action={async () => {
                "use server";
                await signOut({ redirectTo: "/login" });
              }}
            >
              <button type="submit" className={styles.signOut}>
                Sign out
              </button>
            </form>
          </div>
        </header>

        <main className={styles.main}>{children}</main>
      </div>
    </div>
  );
}
