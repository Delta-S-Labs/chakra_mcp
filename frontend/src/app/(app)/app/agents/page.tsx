import Link from "next/link";
import { auth } from "@/auth";
import { listOrgs } from "@/lib/api";
import { listMyAgents, listNetworkAgents } from "@/lib/relay";
import { CreateAgentForm } from "./CreateAgentForm";
import styles from "./agents.module.css";

/**
 * /app/agents - agents you own + a peek at the network.
 *
 * "My agents" reads from the relay's /v1/agents (filtered to accounts
 * you're a member of). The network teaser is the top of /v1/network/agents
 * minus the ones you already own.
 */
export default async function AgentsPage() {
  const session = await auth();
  const token = session?.backendToken;

  let myAgents: Awaited<ReturnType<typeof listMyAgents>> = [];
  let networkAgents: Awaited<ReturnType<typeof listNetworkAgents>> = [];
  let orgs: Awaited<ReturnType<typeof listOrgs>> = [];
  let backendError: string | null = null;

  if (token) {
    try {
      const [m, n, o] = await Promise.all([
        listMyAgents(token),
        listNetworkAgents(token),
        listOrgs(token),
      ]);
      myAgents = m;
      networkAgents = n.filter((a) => !a.is_mine).slice(0, 6);
      orgs = o;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Agents</div>
        <h1 className={styles.title}>Your agents.</h1>
        <p className={styles.body}>
          Register the agents you own here. Each one lives inside an
          account (personal or organization), exposes named capabilities,
          and toggles between private and network-visible. Friendships
          and grants - what determines who can actually invoke what -
          come next.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <CreateAgentForm token={token ?? null} accounts={orgs} />

      <section>
        <h2 className={styles.sectionTitle}>
          Mine <span className={styles.count}>{myAgents.length}</span>
        </h2>
        {myAgents.length === 0 ? (
          <p className={styles.empty}>
            No agents yet. Use the form above to register your first one.
          </p>
        ) : (
          <ul className={styles.list}>
            {myAgents.map((a) => (
              <li key={a.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>
                    {a.display_name}{" "}
                    <span
                      className={`${styles.badge} ${
                        a.visibility === "network" ? styles.badgeOn : ""
                      }`}
                    >
                      {a.visibility}
                    </span>
                  </div>
                  <div className={styles.rowMeta}>
                    <code>{a.account_slug}/{a.slug}</code> ·{" "}
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

      <section>
        <h2 className={styles.sectionTitle}>
          On the network{" "}
          <Link href="/app/agents/network" className={styles.viewAll}>
            view all →
          </Link>
        </h2>
        {networkAgents.length === 0 ? (
          <p className={styles.empty}>
            No public agents on this network yet - be the first to flip
            yours to <code>network</code>.
          </p>
        ) : (
          <ul className={styles.list}>
            {networkAgents.map((a) => (
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
    </div>
  );
}
