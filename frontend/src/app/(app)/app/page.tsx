import Link from "next/link";
import { auth, signOut } from "@/auth";
import styles from "./dashboard.module.css";

/**
 * /app — relay app dashboard.
 *
 * Server component. Reads the session via NextAuth's `auth()` helper.
 * Middleware ensures we never reach this page without a session, but we
 * still defensively check.
 */
export default async function AppDashboard() {
  const session = await auth();
  if (!session?.user) {
    // Should be unreachable due to middleware, but render a clean fallback.
    return (
      <main className={styles.page}>
        <div className={styles.shell}>
          <p className={styles.body}>Not signed in.</p>
        </div>
      </main>
    );
  }

  const { name, email, image } = session.user;

  return (
    <main className={styles.page}>
      <div className={styles.shell}>
        <header className={styles.header}>
          <Link href="/" className={styles.brandmark} aria-label="ChakraMCP home">
            <span className={styles.dot} aria-hidden="true" />
            <span className={styles.brandWord}>ChakraMCP</span>
          </Link>
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

        <section className={styles.welcome}>
          <div className={styles.eyebrow}>Dashboard</div>
          <h1 className={styles.title}>You&apos;re in.</h1>
          <p className={styles.body}>
            Hi {name?.split(" ")[0] ?? "there"}. You&apos;re signed in to the relay
            app. The agent management surfaces — registration, friendships,
            grants, audit — land here once the Rust backend Phase 1 ships.
          </p>
        </section>

        <section className={styles.placeholders}>
          <article className={styles.tile}>
            <div className={styles.tileEyebrow}>Soon</div>
            <h2>Agents</h2>
            <p>Register agents you own. Publish capabilities. Toggle visibility.</p>
            <span className={styles.tileBadge}>Pending Phase 1</span>
          </article>
          <article className={styles.tile}>
            <div className={styles.tileEyebrow}>Soon</div>
            <h2>Friendships</h2>
            <p>Inbound and outbound proposals. Counteroffer, accept, reject.</p>
            <span className={styles.tileBadge}>Pending Phase 1</span>
          </article>
          <article className={styles.tile}>
            <div className={styles.tileEyebrow}>Soon</div>
            <h2>Grants &amp; consent</h2>
            <p>Active directional grants, consent records, revocation history.</p>
            <span className={styles.tileBadge}>Pending Phase 1</span>
          </article>
          <article className={styles.tile}>
            <div className={styles.tileEyebrow}>Soon</div>
            <h2>Audit log</h2>
            <p>Every relay call, every actor context, every outcome.</p>
            <span className={styles.tileBadge}>Pending Phase 1</span>
          </article>
        </section>
      </div>
    </main>
  );
}
