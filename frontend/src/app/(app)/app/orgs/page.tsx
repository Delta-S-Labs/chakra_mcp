import Link from "next/link";
import { auth } from "@/auth";
import { listOrgs } from "@/lib/api";
import { listMyAgents, type Agent } from "@/lib/relay";
import { CreateOrgForm } from "./CreateOrgForm";
import styles from "./orgs.module.css";

export default async function OrgsPage() {
  const session = await auth();
  const token = session?.backendToken;

  let orgs: Awaited<ReturnType<typeof listOrgs>> = [];
  let agents: Awaited<ReturnType<typeof listMyAgents>> = [];
  let backendError: string | null = null;
  if (token) {
    try {
      // Pull both in parallel - agents come from the relay, orgs from
      // the app service. Surface the relay failure quietly: org cards
      // still render, just without an "Agents" sub-list.
      const [o, a] = await Promise.all([
        listOrgs(token),
        listMyAgents(token).catch(() => [] as Agent[]),
      ]);
      orgs = o;
      agents = a;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Backend unavailable.";
    }
  }

  const agentsByAccount = groupAgentsByAccount(agents);

  const personal = orgs.filter((o) => o.account_type === "individual");
  const teams = orgs.filter((o) => o.account_type === "organization");

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Organizations</div>
        <h1 className={styles.title}>Your accounts.</h1>
        <p className={styles.body}>
          Personal accounts are created automatically when you sign up.
          Organizations are for sharing agents, friendships, and grants
          across a team. Owners and admins can invite members; members
          can be promoted later.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <CreateOrgForm token={token ?? null} />

      {personal.length > 0 && (
        <section>
          <h2 className={styles.sectionTitle}>Personal</h2>
          <ul className={styles.cardList}>
            {personal.map((o) => (
              <AccountCard
                key={o.id}
                slug={o.slug}
                displayName={o.display_name}
                role={o.role}
                agents={agentsByAccount.get(o.id) ?? []}
                href={null}
              />
            ))}
          </ul>
        </section>
      )}

      <section>
        <h2 className={styles.sectionTitle}>
          Organizations <span className={styles.count}>{teams.length}</span>
        </h2>
        {teams.length === 0 ? (
          <p className={styles.empty}>
            No organizations yet. Create one above to invite teammates.
          </p>
        ) : (
          <ul className={styles.cardList}>
            {teams.map((o) => (
              <AccountCard
                key={o.id}
                slug={o.slug}
                displayName={o.display_name}
                role={o.role}
                agents={agentsByAccount.get(o.id) ?? []}
                href={`/app/orgs/${o.slug}`}
              />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function groupAgentsByAccount(agents: Agent[]): Map<string, Agent[]> {
  const out = new Map<string, Agent[]>();
  for (const a of agents) {
    const list = out.get(a.account_id);
    if (list) list.push(a);
    else out.set(a.account_id, [a]);
  }
  for (const list of out.values()) {
    list.sort((x, y) => x.display_name.localeCompare(y.display_name));
  }
  return out;
}

function AccountCard({
  slug,
  displayName,
  role,
  agents,
  href,
}: {
  slug: string;
  displayName: string;
  role: string;
  agents: Agent[];
  href: string | null;
}) {
  return (
    <li className={styles.card}>
      <div className={styles.cardHead}>
        <div>
          <div className={styles.rowName}>{displayName}</div>
          <div className={styles.rowMeta}>
            <code>{slug}</code> · {role}
          </div>
        </div>
        {href && (
          <Link className={styles.openLink} href={href}>
            Open →
          </Link>
        )}
      </div>

      <div className={styles.cardAgents}>
        <div className={styles.cardAgentsHead}>
          <span>Agents</span>
          <span className={styles.count}>{agents.length}</span>
        </div>
        {agents.length === 0 ? (
          <p className={styles.subEmpty}>
            No agents under this account yet.{" "}
            <Link href="/app/agents">Register one →</Link>
          </p>
        ) : (
          <ul className={styles.agentList}>
            {agents.map((a) => (
              <li key={a.id} className={styles.agentRow}>
                <Link href={`/app/agents/${a.id}`} className={styles.agentLink}>
                  <span className={styles.agentName}>{a.display_name}</span>
                  <code className={styles.agentSlug}>
                    {a.account_slug}/{a.slug}
                  </code>
                </Link>
                <div className={styles.agentMeta}>
                  <span
                    className={`${styles.visibilityBadge} ${
                      a.visibility === "network" ? styles.visibilityBadgeOn : ""
                    }`}
                  >
                    {a.visibility}
                  </span>
                  <span className={styles.agentMetaCount}>
                    {a.capability_count}{" "}
                    {a.capability_count === 1 ? "capability" : "capabilities"}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </li>
  );
}
