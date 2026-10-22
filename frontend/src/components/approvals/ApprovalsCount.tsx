import { waitingApprovals } from "./waiting";
import styles from "./approvals.module.css";

/** The nav's count of requests waiting for approval; nothing when none. */
export async function ApprovalsCount() {
  const n = await waitingApprovals();
  if (!n) return null;
  return (
    <span className={styles.count} aria-label={`${n} waiting`}>
      {n}
    </span>
  );
}
