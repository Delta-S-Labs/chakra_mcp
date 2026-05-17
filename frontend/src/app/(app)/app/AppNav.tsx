"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import styles from "./shell.module.css";

type Tab = { label: string; href: string; exact?: boolean };

/**
 * Top-bar tabs.
 *
 * Settings-ish surfaces (API keys, Audit, Pair agent, Admin) live under
 * the user-menu dropdown in `layout.tsx`, not here — they used to be in
 * this list and the row overflowed past ~8 tabs.
 */
const tabs: Tab[] = [
  { label: "Dashboard", href: "/app", exact: true },
  { label: "Agents", href: "/app/agents" },
  { label: "Friendships", href: "/app/friendships" },
  { label: "Grants", href: "/app/grants" },
  { label: "Inbox", href: "/app/inbox" },
];

export function AppNav() {
  const pathname = usePathname();
  const isActive = (t: Tab) =>
    t.exact ? pathname === t.href : pathname === t.href || pathname.startsWith(`${t.href}/`);

  return (
    <nav className={styles.nav} aria-label="App navigation">
      {tabs.map((t) => (
        <Link
          key={t.href}
          href={t.href}
          className={`${styles.navLink} ${isActive(t) ? styles.navLinkActive : ""}`}
        >
          {t.label}
        </Link>
      ))}
    </nav>
  );
}
