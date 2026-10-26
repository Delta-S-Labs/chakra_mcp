"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import styles from "./shell.module.css";

/** The Calls tab, shown only when the relay has calls on (`CallsTab`). */
export function CallsNavLink() {
  const pathname = usePathname();
  const active = pathname === "/app/calls" || pathname.startsWith("/app/calls/");
  return (
    <Link href="/app/calls" className={`${styles.navLink} ${active ? styles.navLinkActive : ""}`}>
      Calls
    </Link>
  );
}
