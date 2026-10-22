"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import styles from "./approvals.module.css";

/** The banner itself; hidden on the approvals pages, which list them. */
export function ApprovalsBannerView({ count }: { count: number }) {
  const pathname = usePathname();
  if (pathname.startsWith("/app/approvals")) return null;
  return (
    <div className={styles.banner} role="status" aria-label="Approvals">
      <span>
        {count === 1
          ? "A request to one of your agents is waiting for your approval."
          : `${count} requests to your agents are waiting for your approval.`}
      </span>{" "}
      <Link className={styles.action} href="/app/approvals">
        Review
      </Link>
    </div>
  );
}
