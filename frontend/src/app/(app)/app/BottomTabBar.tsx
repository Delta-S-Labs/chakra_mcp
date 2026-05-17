"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import styles from "./shell.module.css";

type Tab = {
  label: string;
  href: string;
  exact?: boolean;
  icon: React.ReactNode;
};

/**
 * Bottom tab bar — phone only.
 *
 * Hidden at >540px (CSS in shell.module.css). At ≤540px the top swipe-strip
 * nav is hidden and this bar takes its place; main content gets matching
 * bottom-padding so cards never hide under the bar.
 *
 * The 5 chosen routes are the daily-driver ones: Dashboard / Agents / Friends
 * / Grants / Inbox. Orgs / API keys / Audit / Pair / Usage / Admin / Docs
 * all live in the UserMenu — they're settings-ish, not daily flow.
 */
const tabs: Tab[] = [
  {
    label: "Home",
    href: "/app",
    exact: true,
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M3 11.5 12 4l9 7.5V20a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1Z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    label: "Agents",
    href: "/app/agents",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect
          x="4"
          y="7"
          width="16"
          height="12"
          rx="2.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
        />
        <path d="M12 4v3" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        <circle cx="9" cy="13" r="1.3" fill="currentColor" />
        <circle cx="15" cy="13" r="1.3" fill="currentColor" />
      </svg>
    ),
  },
  {
    label: "Friends",
    href: "/app/friendships",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="9" cy="9" r="3" fill="none" stroke="currentColor" strokeWidth="1.6" />
        <circle cx="16.5" cy="10.5" r="2.4" fill="none" stroke="currentColor" strokeWidth="1.6" />
        <path
          d="M3 19c.4-2.5 2.8-4.5 6-4.5s5.6 2 6 4.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
        <path
          d="M14 19c.3-1.9 2-3.5 4-3.5s3.7 1.6 4 3.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    ),
  },
  {
    label: "Grants",
    href: "/app/grants",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="8" cy="12" r="3.5" fill="none" stroke="currentColor" strokeWidth="1.6" />
        <path
          d="M11.5 12H21M18 12v3M15 12v2"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    ),
  },
  {
    label: "Inbox",
    href: "/app/inbox",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M4 6h16l-1.6 8.2a2 2 0 0 1-2 1.6H7.6a2 2 0 0 1-2-1.6L4 6Z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
        <path
          d="M4 14h4l1.5 2h5L16 14h4"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
];

export function BottomTabBar() {
  const pathname = usePathname();
  const isActive = (t: Tab) =>
    t.exact ? pathname === t.href : pathname === t.href || pathname.startsWith(`${t.href}/`);

  return (
    <nav className={styles.bottomBar} aria-label="App navigation (mobile)">
      {tabs.map((t) => (
        <Link
          key={t.href}
          href={t.href}
          className={`${styles.bottomTab} ${isActive(t) ? styles.bottomTabActive : ""}`}
          aria-current={isActive(t) ? "page" : undefined}
        >
          <span className={styles.bottomTabIcon}>{t.icon}</span>
          <span className={styles.bottomTabLabel}>{t.label}</span>
        </Link>
      ))}
    </nav>
  );
}
