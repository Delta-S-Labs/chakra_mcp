import Link from "next/link";
import { auth } from "@/auth";
import { listNetworkAgents } from "@/lib/relay";
import styles from "../agents.module.css";

/**
 * /app/agents/network - discovery view of every agent on the relay
 * that has flipped its visibility to `network`.
 */
export default async function NetworkAgentsPage() {
  const session = await auth();
  const token = session?.backendToken;

  let agents: Awaited<ReturnType<typeof listNetworkAgents>> = [];
  let backendError: string | null = null;
  if (token) {
    try {
      agents = await listNetworkAgents(token);
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  const others = agents.filter((a) => !a.is_mine);
  const mine = agents.filter((a) => a.is_mine);

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/agents" className={styles.backLink}>
            ← Agents
          </Link>
        </div>
        <h1 className={styles.title}>The network.</h1>
        <p className={styles.body}>
          Every network-visible agent on this relay. Friendships and grants
          gate who can actually invoke whom - for now this is just a
          directory you can browse to see what&apos;s out there.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <section>
        <h2 className={styles.sectionTitle}>
          Others <span className={styles.count}>{others.length}</span>
        </h2>
        {others.length === 0 ? (
          <p className={styles.empty}>
            No other agents on the network yet. Encourage someone to flip
            theirs to <code>network</code>.
          </p>
        ) : (
          <ul className={styles.list}>
            {others.map((a) => (
              <li key={a.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>{a.display_name}</div>
                  <div className={styles.rowMeta}>
                    by <strong>{a.account_display_name}</strong> ·{" "}
                    {a.capability_count}{" "}
                    {a.capability_count === 1 ? "capability" : "capabilities"}
                  </div>
                </div>
                <Link className={styles.openLink} href={`/app/agents/${a.id}`}>
                  Open →
                </Link>
              </li>
            ))}
          </ul>
        )}
      </section>

      {mine.length > 0 && (
        <section>
          <h2 className={styles.sectionTitle}>
            Yours on the network <span className={styles.count}>{mine.length}</span>
          </h2>
          <ul className={styles.list}>
            {mine.map((a) => (
              <li key={a.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>{a.display_name}</div>
                  <div className={styles.rowMeta}>
                    <code>
                      {a.account_slug}/{a.slug}
                    </code>{" "}
                    · {a.capability_count}{" "}
                    {a.capability_count === 1 ? "capability" : "capabilities"}
                  </div>
                </div>
                <Link className={styles.openLink} href={`/app/agents/${a.id}`}>
                  Open →
                </Link>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
