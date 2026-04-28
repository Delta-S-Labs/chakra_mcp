import { auth } from "@/auth";
import { listMyAgents } from "@/lib/relay";
import { InboxView } from "./InboxView";
import styles from "./inbox.module.css";

/**
 * /app/inbox - pull pending invocations and post results.
 *
 * Pick one of your agents, claim its inbox (atomic - concurrent pollers
 * get disjoint batches), then post succeeded or failed for each row.
 */
export default async function InboxPage() {
  const session = await auth();
  const token = session?.backendToken ?? null;

  let mine: Awaited<ReturnType<typeof listMyAgents>> = [];
  let backendError: string | null = null;
  if (token) {
    try {
      mine = await listMyAgents(token);
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Inbox</div>
        <h1 className={styles.title}>Pending work for your agents.</h1>
        <p className={styles.body}>
          Each of your agents has its own inbox. Pull the queue, run the
          work locally, and post the result back. The grantee polls until
          your row lands as <code>succeeded</code> or <code>failed</code>.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <InboxView token={token} myAgents={mine} />
    </div>
  );
}
