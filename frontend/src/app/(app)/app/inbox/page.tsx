import { auth } from "@/auth";
import Link from "next/link";
import { callsEnabled, listInvocations, listMyAgents, type Invocation } from "@/lib/relay";
import { redirectIfUnauthorized } from "@/lib/session-expired";
import { InboxView } from "./InboxView";
import styles from "./inbox.module.css";

/**
 * /app/inbox - the full invocation queue for each of your agents.
 *
 * Pick one of your agents to see every call directed at it - pending,
 * in-flight (already claimed, e.g. waiting on a human-in-the-loop
 * answer), and finished (succeeded / failed). "Pull inbox" claims the
 * oldest pending rows (atomic - concurrent pollers get disjoint
 * batches) so you can post a result.
 */
export default async function InboxPage() {
  const session = await auth();
  const token = session?.backendToken ?? null;

  let mine: Awaited<ReturnType<typeof listMyAgents>> = [];
  let initialItems: Invocation[] = [];
  let backendError: string | null = null;
  let callsOn = false;
  if (token) {
    try {
      [mine, callsOn] = await Promise.all([listMyAgents(token), callsEnabled(token)]);
      if (mine.length > 0) {
        // Everything directed AT the first agent (it's the granter),
        // regardless of status. Soft dependency — fall back to empty.
        try {
          initialItems = await listInvocations(token, {
            direction: "inbound",
            agent_id: mine[0].id,
          });
        } catch {
          /* leave empty; client can refresh */
        }
      }
    } catch (err) {
      redirectIfUnauthorized(err, "/app/inbox");
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
        {callsOn && (
          <p className={styles.body}>
            Conversations between agents are under <Link href="/app/calls">Calls</Link>.
          </p>
        )}
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <InboxView
        token={token}
        myAgents={mine}
        initialItems={initialItems}
      />
    </div>
  );
}
