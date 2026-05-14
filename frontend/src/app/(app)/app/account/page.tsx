import Link from "next/link";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getMe } from "@/lib/api";
import { listMyAgents, type Agent } from "@/lib/relay";
import styles from "../orgs/orgs.module.css";

/**
 * /app/account — the user's personal account.
 *
 * Mirror of the org-detail page, minus member/invite UI. A personal
 * account always has exactly one member (you), so there's nothing to
 * manage there. The interesting part is the agents currently parented
 * to this account — that's the surface where you'd land after
 * "unregister from org → move to personal".
 *
 * If the backend's /v1/me responds with no individual-type membership
 * (shouldn't happen for normal sign-ups, but possible for an admin
 * bootstrap state), redirect to /app/orgs so the user has somewhere to
 * go.
 */
export default async function AccountPage() {
  const session = await auth();
  const token = session?.backendToken;

  if (!token) {
    return (
      <div className={styles.page}>
        <header className={styles.head}>
          <div className="eyebrow">My account</div>
          <h1 className={styles.title}>Sign in to continue.</h1>
        </header>
      </div>
    );
  }

  let backendError: string | null = null;
  let personal: Awaited<ReturnType<typeof getMe>>["memberships"][number] | null = null;
  try {
    const me = await getMe(token);
    personal = me.memberships.find((m) => m.account_type === "individual") ?? null;
  } catch (err) {
    backendError = err instanceof Error ? err.message : "Backend unavailable.";
  }

  if (!backendError && !personal) {
    // No personal account on file — bounce to orgs so the user can
    // see whatever they do have.
    redirect("/app/orgs");
  }

  let agents: Agent[] = [];
  let relayError: string | null = null;
  if (personal) {
    try {
      const all = await listMyAgents(token);
      agents = all
        .filter((a) => a.account_id === personal.account_id)
        .sort((x, y) => x.display_name.localeCompare(y.display_name));
    } catch (err) {
      relayError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">My account</div>
        <h1 className={styles.title}>{personal?.display_name ?? "Your account"}</h1>
        <p className={styles.body}>
          <code>{personal?.slug}</code> · personal account · one member, you.
          This is where agents land when you unregister them from an
          organization. Members and invites do not apply here.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <section>
        <h2 className={styles.sectionTitle}>
          Agents <span className={styles.count}>{agents.length}</span>
          <Link href="/app/agents" className={styles.viewAll}>
            Create new agent →
          </Link>
        </h2>

        {relayError && (
          <p className={styles.empty}>
            Couldn&apos;t reach the relay: {relayError}
          </p>
        )}

        {!relayError && agents.length === 0 && (
          <p className={styles.empty}>
            No agents under this account yet.{" "}
            <Link href="/app/agents">Register your first one →</Link>
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
