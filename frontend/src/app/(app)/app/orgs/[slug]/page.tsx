import Link from "next/link";
import { notFound } from "next/navigation";
import { auth } from "@/auth";
import { getOrg, listMembers } from "@/lib/api";
import { listMyAgents, type Agent } from "@/lib/relay";
import { InviteForm } from "./InviteForm";
import { MoveAgentButton } from "./MoveAgentButton";
import { DeleteOrgButton } from "./DeleteOrgButton";
import styles from "../orgs.module.css";

export default async function OrgDetailsPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const session = await auth();
  const token = session?.backendToken;
  if (!token) notFound();

  let org: Awaited<ReturnType<typeof getOrg>>;
  let members: Awaited<ReturnType<typeof listMembers>>;
  try {
    [org, members] = await Promise.all([getOrg(token, slug), listMembers(token, slug)]);
  } catch {
    notFound();
  }

  // Agents under this org. The relay's /v1/agents lists every agent
  // across all the accounts the caller is a member of, so we filter
  // client-side by account_id. If the relay is down we still render
  // the members section - just without agents.
  let allMyAgents: Agent[] = [];
  let relayError: string | null = null;
  try {
    allMyAgents = await listMyAgents(token);
  } catch (err) {
    relayError = err instanceof Error ? err.message : "Relay unavailable.";
  }
  const agents = allMyAgents
    .filter((a) => a.account_id === org.id)
    .sort((x, y) => x.display_name.localeCompare(y.display_name));

  const canInvite = org.role === "owner" || org.role === "admin";
  const myEmail = session?.user?.email;

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          {org.account_type === "organization" ? "Organization" : "Personal"}
        </div>
        <h1 className={styles.title}>{org.display_name}</h1>
        <p className={styles.body}>
          <code>{org.slug}</code> · you are a <strong>{org.role}</strong>.
        </p>
      </header>

      <section>
        <h2 className={styles.sectionTitle}>
          Agents <span className={styles.count}>{agents.length}</span>
          <Link href="/app/agents" className={styles.viewAll}>
            register →
          </Link>
        </h2>

        {relayError && (
          <p className={styles.empty}>
            Couldn&apos;t reach the relay: {relayError}
          </p>
        )}

        {!relayError && agents.length === 0 && (
          <p className={styles.empty}>
            No agents registered under <strong>{org.display_name}</strong> yet.
            Head over to <Link href="/app/agents">Agents</Link> to register one.
          </p>
        )}

        {agents.length > 0 && (
          <ul className={styles.list}>
            {agents.map((a) => (
              <li key={a.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>
                    {a.display_name}{" "}
                    <span
                      className={`${styles.visibilityBadge} ${
                        a.visibility === "network" ? styles.visibilityBadgeOn : ""
                      }`}
                    >
                      {a.visibility}
                    </span>
                  </div>
                  <div className={styles.rowMeta}>
                    <code>
                      {a.account_slug}/{a.slug}
                    </code>{" "}
                    · {a.capability_count}{" "}
                    {a.capability_count === 1 ? "capability" : "capabilities"}
                    {a.description ? ` · ${a.description}` : ""}
                  </div>
                </div>
                <div className={styles.rowActions}>
                  <MoveAgentButton
                    token={token}
                    agentId={a.id}
                    agentDisplayName={a.display_name}
                    agentSlug={a.slug}
                    fromAccountSlug={a.account_slug}
                  />
                  <Link className={styles.openLink} href={`/app/agents/${a.id}`}>
                    Open →
                  </Link>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <h2 className={styles.sectionTitle}>
          Members <span className={styles.count}>{members.length}</span>
        </h2>
        <ul className={styles.list}>
          {members.map((m) => (
            <li key={m.user_id} className={styles.row}>
              <div className={styles.memberLeft}>
                {m.avatar_url && (
                  // eslint-disable-next-line @next/next/no-img-element
                  <img src={m.avatar_url} alt="" className={styles.avatar} />
                )}
                <div>
                  <div className={styles.rowName}>{m.display_name}</div>
                  <div className={styles.rowMeta}>
                    <code>{m.email}</code>
                    {m.email === myEmail && " · you"}
                  </div>
                </div>
              </div>
              <span className={styles.roleBadge}>{m.role}</span>
            </li>
          ))}
        </ul>
      </section>

      {canInvite && org.account_type === "organization" && (
        <InviteForm slug={org.slug} token={token} />
      )}

      {org.account_type === "organization" && org.role === "owner" && (
        <DeleteOrgButton
          token={token}
          slug={org.slug}
          displayName={org.display_name}
          agentCount={agents.length}
        />
      )}
    </div>
  );
}
