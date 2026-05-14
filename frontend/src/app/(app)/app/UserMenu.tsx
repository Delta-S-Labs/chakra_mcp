import Link from "next/link";
import { signOutAndRedirect } from "@/lib/auth-actions";
import styles from "./shell.module.css";

/**
 * Avatar / dropdown in the top bar.
 *
 * Rendered as a `<details>` so the menu state lives in the DOM — no
 * client component, no useState. The native disclosure triangle is
 * hidden with `summary { list-style: none }` and an
 * `::-webkit-details-marker` reset in shell.module.css; what the user
 * sees is a styled chip.
 *
 * Contents:
 *   - User identity (name + email) at the top
 *   - Secondary nav (API keys, Audit, Pair agent, Admin)
 *   - Sign out form — delegates to `signOutAndRedirect` (see
 *     `src/lib/auth-actions.ts`) which does the belt-and-braces
 *     cookie purge needed under NextAuth v5 beta + Next.js 16. The
 *     naïve `signOut({ redirect: false }) → revalidatePath → redirect`
 *     pattern leaves the `__Secure-authjs.session-token` cookie alive
 *     in some edge paths — auto re-signs the user in on next visit.
 */
export function UserMenu({
  name,
  email,
  image,
  isAdmin,
}: {
  name?: string | null;
  email?: string | null;
  image?: string | null;
  isAdmin: boolean;
}) {
  const initial = (name?.trim()?.[0] ?? email?.trim()?.[0] ?? "?").toUpperCase();

  return (
    <details className={styles.userMenu}>
      <summary className={styles.userTrigger} aria-label="Account menu">
        {image ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img src={image} alt="" className={styles.avatar} />
        ) : (
          <span className={styles.avatarFallback} aria-hidden="true">
            {initial}
          </span>
        )}
        <span className={styles.userTriggerMeta}>
          <span className={styles.userTriggerName}>
            {name ?? "Signed in"}
          </span>
        </span>
        <span className={styles.userTriggerCaret} aria-hidden="true">
          ▾
        </span>
      </summary>

      <div className={styles.userDropdown} role="menu">
        <div className={styles.userDropdownIdentity}>
          <div className={styles.userDropdownName}>{name ?? "Signed in"}</div>
          {email && <div className={styles.userDropdownEmail}>{email}</div>}
        </div>

        <div className={styles.userDropdownSection}>
          <Link className={styles.userDropdownItem} href="/app/api-keys">
            API keys
          </Link>
          <Link className={styles.userDropdownItem} href="/app/pair">
            Pair agent
          </Link>
          <Link className={styles.userDropdownItem} href="/app/audit">
            Audit
          </Link>
          {isAdmin && (
            <Link className={styles.userDropdownItem} href="/app/admin">
              Admin
            </Link>
          )}
        </div>

        <form
          className={styles.userDropdownSection}
          action={signOutAndRedirect}
        >
          <button type="submit" className={styles.userDropdownSignOut}>
            Sign out
          </button>
        </form>
      </div>
    </details>
  );
}
