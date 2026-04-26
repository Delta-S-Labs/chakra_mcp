"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import styles from "./shell.module.css";

type Tab = { label: string; href: string; exact?: boolean };

const tabs: Tab[] = [
  { label: "Dashboard", href: "/app", exact: true },
  { label: "Orgs", href: "/app/orgs" },
  { label: "Agents", href: "/app/agents" },
  { label: "Friendships", href: "/app/friendships" },
  { label: "Grants", href: "/app/grants" },
  { label: "Inbox", href: "/app/inbox" },
  { label: "Audit", href: "/app/audit" },
  { label: "API keys", href: "/app/api-keys" },
];

const adminTab: Tab = { label: "Admin", href: "/app/admin" };

export function AppNav({ isAdmin }: { isAdmin: boolean }) {
  const pathname = usePathname();
  const visible: Tab[] = isAdmin ? [...tabs, adminTab] : tabs;
  const isActive = (t: Tab) =>
    t.exact ? pathname === t.href : pathname === t.href || pathname.startsWith(`${t.href}/`);

  return (
    <nav className={styles.nav} aria-label="App navigation">
      {visible.map((t) => (
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
