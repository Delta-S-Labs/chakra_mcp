import { auth } from "@/auth";
import { listMyAgents, listInvocations, listAuditEvents, type AuditEvent } from "@/lib/relay";
import { AuditList } from "./AuditList";
import { LocalTime } from "./LocalTime";
import styles from "./audit.module.css";

/**
 * /app/audit - every relay invocation involving any of your agents.
 *
 * Direction tabs (relative to the agent picker):
 *   "Outbound" = the focal agent invoked someone else.
 *   "Inbound"  = the focal agent served a call from someone else.
 * When the picker is "All my agents" the tabs are account-scoped:
 *   any of my agents on the corresponding side of the wire.
 *
 * Pre-flight rejections (no grant, expired, missing endpoint) and
 * downstream failures both land here so the trail is complete.
 */
export default async function AuditPage() {
  const session = await auth();
  const token = session?.backendToken;

  let invocations: Awaited<ReturnType<typeof listInvocations>> = [];
  let agents: Awaited<ReturnType<typeof listMyAgents>> = [];
  let writeTrail: AuditEvent[] = [];
  let backendError: string | null = null;
  if (token) {
    try {
      const [inv, ag, audit] = await Promise.all([
        listInvocations(token, { direction: "all" }),
        listMyAgents(token),
        listAuditEvents(token, { limit: 50 }),
      ]);
      invocations = inv;
      agents = ag;
      writeTrail = audit.events;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Audit log</div>
        <h1 className={styles.title}>Every call, every outcome.</h1>
        <p className={styles.body}>
          One row per relay invocation involving an agent on either side
          of the wire. Pick an agent below to focus the trail on what it
          sent (Outbound) versus what came in to it (Inbound).
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <AuditList items={invocations} agents={agents} />

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Write trail</h2>
        <p className={styles.body}>
          Every create, update, delete, and state change across your agents,
          capabilities, friendships, grants, and reviews — whoever did it,
          whenever. Read-only actions aren&apos;t logged here (see Usage).
        </p>
        {writeTrail.length === 0 ? (
          <div className={styles.empty}>No write events yet.</div>
        ) : (
          <ul className={styles.evtList}>
            {writeTrail.map((e) => (
              <li key={e.id} className={styles.evt}>
                <span className={styles.evtAction}>{e.action}</span>
                <span className={styles.evtSummary}>{e.summary}</span>
                <span className={styles.evtTime}>
                  <LocalTime iso={e.created_at} />
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
