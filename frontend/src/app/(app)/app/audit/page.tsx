import { auth } from "@/auth";
import { listInvocations } from "@/lib/relay";
import { AuditList } from "./AuditList";
import styles from "./audit.module.css";

/**
 * /app/audit - every relay invocation involving any of your agents.
 *
 * "Outbound" = your agent served the call.
 * "Inbound"  = your agent did the calling.
 * Pre-flight rejections (no grant, expired, missing endpoint) and
 * downstream failures both land here so the trail is complete.
 */
export default async function AuditPage() {
  const session = await auth();
  const token = session?.backendToken;

  let invocations: Awaited<ReturnType<typeof listInvocations>> = [];
  let backendError: string | null = null;
  if (token) {
    try {
      invocations = await listInvocations(token, { direction: "all" });
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
          of the wire. Pre-flight rejections (no grant, expired, missing
          endpoint) and downstream failures both show up - silence here
          really does mean nothing happened.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <AuditList items={invocations} />
    </div>
  );
}
