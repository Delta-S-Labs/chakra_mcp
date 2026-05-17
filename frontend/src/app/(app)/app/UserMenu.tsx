"use client";

import { useEffect, useRef } from "react";
import Link from "next/link";
import { signOutAndRedirect } from "@/lib/auth-actions";
import styles from "./shell.module.css";

/**
 * Avatar / dropdown in the top bar.
 *
 * Rendered as a `<details>` so the menu state lives in the DOM — no
 * useState for the open/closed bit, which keeps the menu functional
 * even before hydration. The native disclosure triangle is hidden
 * with `summary { list-style: none }` and an
 * `::-webkit-details-marker` reset in shell.module.css; what the user
 * sees is a styled chip.
 *
 * # Click-outside close
 *
 * `<details>` natively toggles on `<summary>` click but does NOT
 * close when the user clicks elsewhere on the page. Without that,
 * the dropdown stays open until the user clicks the chip a second
 * time or navigates away — annoying enough that this used to be a
 * filed bug. Fixed here with a global `pointerdown` listener that
 * unsets the `open` attribute when the click target isn't inside
 * the menu. The listener attaches once per mount.
 *
 * # Contents:
 *   - User identity (name + email) at the top
 *   - In-app links (API keys, Audit, Pair agent, Usage, Admin)
 *   - Docs link out to the public docs (separate section)
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
  const detailsRef = useRef<HTMLDetailsElement | null>(null);

  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      const el = detailsRef.current;
      if (!el || !el.open) return;
      const target = e.target as Node | null;
      if (target && !el.contains(target)) {
        el.open = false;
      }
    };
    // pointerdown fires before any click handler, so a click on a
    // dropdown item (e.g. a Link) still navigates correctly — the
    // listener only triggers for clicks *outside* the menu element.
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, []);

  return (
    <details ref={detailsRef} className={styles.userMenu}>
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
          <Link className={styles.userDropdownItem} href="/app/account">
            My account
          </Link>
          <Link className={styles.userDropdownItem} href="/app/orgs">
            Orgs
          </Link>
          <Link className={styles.userDropdownItem} href="/app/api-keys">
            API keys
          </Link>
          <Link className={styles.userDropdownItem} href="/app/pair">
            Pair agent
          </Link>
          <Link className={styles.userDropdownItem} href="/app/usage">
            Usage
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

        <div className={styles.userDropdownSection}>
          <Link className={styles.userDropdownItem} href="/docs">
            Docs
          </Link>
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
