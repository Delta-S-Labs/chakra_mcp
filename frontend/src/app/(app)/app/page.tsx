import Link from "next/link";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getMe } from "@/lib/api";
import {
  listFriendships,
  listGrants,
  listInvocations,
  listMyAgents,
  type Invocation,
  type InvocationStatus,
} from "@/lib/relay";
import styles from "./dashboard.module.css";

/**
 * /app - relay app dashboard.
 *
 * Reads the session from NextAuth, then pulls the canonical user +
 * memberships from `/v1/me`. If the backend reports
 * `survey_required: true`, hard-redirects to /app/welcome.
 *
 * Beyond identity, this view summarises the user's relay state:
 * agents, in-flight friendships, active grants, pending invocations,
 * and the most recent audit rows. Everything is server-rendered from
 * the existing list endpoints - no charting libs, no new backend.
 */
export default async function AppDashboard() {
  const session = await auth();
  const token = session?.backendToken;
  const name = session?.user?.name;

  let memberships: Awaited<ReturnType<typeof getMe>>["memberships"] = [];
  let surveyRequired = false;
  let backendError: string | null = null;
  let agents: Awaited<ReturnType<typeof listMyAgents>> = [];
  let friendships: Awaited<ReturnType<typeof listFriendships>> = [];
  let grants: Awaited<ReturnType<typeof listGrants>> = [];
  let invocations: Awaited<ReturnType<typeof listInvocations>> = [];
  let relayError: string | null = null;

  if (token) {
    try {
      const me = await getMe(token);
      surveyRequired = me.survey_required;
      memberships = me.memberships;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Backend unavailable.";
    }

    try {
      [agents, friendships, grants, invocations] = await Promise.all([
        listMyAgents(token),
        listFriendships(token, { direction: "all" }),
        listGrants(token, { direction: "all" }),
        listInvocations(token, { direction: "all" }),
      ]);
    } catch (err) {
      relayError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  } else {
    backendError = "No backend token in session - sign in again.";
  }

  if (surveyRequired) {
    redirect("/app/welcome");
  }

  const personal = memberships.find((m) => m.account_type === "individual");
  const orgs = memberships.filter((m) => m.account_type === "organization");

  const agentCount = agents.length;
  const networkAgentCount = agents.filter((a) => a.visibility === "network").length;
  const activeGrants = grants.filter((g) => g.status === "active");
  const outboundGrants = activeGrants.filter((g) => g.i_granted).length;
  const inboundGrants = activeGrants.filter((g) => g.i_received).length;
  const pendingFriendships = friendships.filter(
    (f) => f.status === "proposed" && f.i_received,
  ).length;
  const pendingInvocations = invocations.filter(
    (i) => i.status === "pending" || i.status === "in_progress",
  ).length;
  const recentInvocations = [...invocations]
    .sort((a, b) => b.created_at.localeCompare(a.created_at))
    .slice(0, 5);

  return (
    <>
      <section className={styles.welcome}>
        <div className={styles.eyebrow}>Dashboard</div>
        <h1 className={styles.title}>You&apos;re in.</h1>
        <p className={styles.body}>
          Hi {name?.split(" ")[0] ?? "there"}. You&apos;re signed in to the relay
          app.{session?.user?.is_admin ? " Admin tab is available from the profile menu." : ""}
        </p>
      </section>

      {backendError && (
        <section className={styles.notice}>
          <strong>Backend says:</strong> {backendError}
          <p>
            Make sure <code>chakramcp-app</code> is running locally
            (<code>task db:up &amp;&amp; task dev:backend</code>) and
            <code> NEXT_PUBLIC_APP_API_URL</code> matches.
          </p>
        </section>
      )}

      {relayError && (
        <section className={styles.notice}>
          <strong>Relay says:</strong> {relayError}
          <p>Stats below may be incomplete until the relay is back.</p>
        </section>
      )}

      <section className={styles.statGrid}>
        <StatCard
          label="Agents"
          value={agentCount}
          hint={
            agentCount === 0
              ? "Register your first agent"
              : `${networkAgentCount} on the network`
          }
          href="/app/agents"
        />
        <StatCard
          label="Active grants"
          value={activeGrants.length}
          hint={`${outboundGrants} out · ${inboundGrants} in`}
          href="/app/grants"
        />
        <StatCard
          label="Pending friendships"
          value={pendingFriendships}
          hint={
            pendingFriendships === 0
              ? "Nothing inbound right now"
              : `${pendingFriendships === 1 ? "1 proposal" : `${pendingFriendships} proposals`} awaiting you`
          }
          href="/app/friendships"
        />
        <StatCard
          label="In-flight inbox"
          value={pendingInvocations}
          hint={
            pendingInvocations === 0
              ? "All caught up"
              : `${pendingInvocations === 1 ? "1 call" : `${pendingInvocations} calls`} pending`
          }
          href="/app/inbox"
        />
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHead}>
          <h2 className={styles.sectionTitle}>Recent activity</h2>
          <Link href="/app/audit" className={styles.sectionLink}>
            Full audit log →
          </Link>
        </div>
        {recentInvocations.length === 0 ? (
          <p className={styles.empty}>
            No relay calls yet. They&apos;ll show up here once a friend invokes
            one of your capabilities - or you invoke one of theirs from{" "}
            <Link href="/app/grants">Grants</Link>.
          </p>
        ) : (
          <table className={styles.activityTable}>
            <thead>
              <tr>
                <th>When</th>
                <th>Capability</th>
                <th>From → To</th>
                <th>Status</th>
                <th className={styles.numericCol}>Elapsed</th>
              </tr>
            </thead>
            <tbody>
              {recentInvocations.map((row) => (
                <ActivityRow key={row.id} row={row} />
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Your accounts</h2>
        <div className={styles.accountList}>
          {personal && (
            <article className={styles.accountTile}>
              <div className={styles.tileEyebrow}>Personal</div>
              <h3>{personal.display_name}</h3>
              <p className={styles.tileMeta}>
                <code>{personal.slug}</code> · {personal.role}
              </p>
            </article>
          )}
          {orgs.map((o) => (
            <article key={o.account_id} className={styles.accountTile}>
              <div className={styles.tileEyebrow}>Organization</div>
              <h3>{o.display_name}</h3>
              <p className={styles.tileMeta}>
                <code>{o.slug}</code> · {o.role}
              </p>
            </article>
          ))}
          {!personal && orgs.length === 0 && !backendError && (
            <article className={styles.accountTile}>
              <p>No accounts yet - try signing out and back in to bootstrap.</p>
            </article>
          )}
        </div>
      </section>
    </>
  );
}

function StatCard({
  label,
  value,
  hint,
  href,
}: {
  label: string;
  value: number;
  hint: string;
  href: string;
}) {
  return (
    <Link href={href} className={styles.statCard}>
      <div className={styles.statLabel}>{label}</div>
      <div className={styles.statValue}>{value}</div>
      <div className={styles.statHint}>{hint}</div>
    </Link>
  );
}

function ActivityRow({ row }: { row: Invocation }) {
  return (
    <tr>
      <td>
        <time dateTime={row.created_at} title={new Date(row.created_at).toLocaleString()}>
          {formatRelative(row.created_at)}
        </time>
      </td>
      <td>
        <code className={styles.capCode}>{row.capability_name}</code>
      </td>
      <td className={styles.partyCell}>
        <span>{row.grantee_display_name ?? "—"}</span>
        <span className={styles.arrow}>→</span>
        <span>{row.granter_display_name ?? "—"}</span>
      </td>
      <td>
        <StatusBadge status={row.status} />
      </td>
      <td className={styles.numericCol}>
        {row.elapsed_ms > 0 ? `${row.elapsed_ms}ms` : "—"}
      </td>
    </tr>
  );
}

function StatusBadge({ status }: { status: InvocationStatus }) {
  const cls =
    status === "succeeded"
      ? styles.badgeOk
      : status === "pending" || status === "in_progress" || status === "rejected"
      ? styles.badgeWarn
      : styles.badgeBad;
  return <span className={`${styles.badge} ${cls}`}>{status}</span>;
}

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const diff = Date.now() - then;
  const sec = Math.round(diff / 1000);
  if (sec < 45) return `${Math.max(sec, 1)}s ago`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  if (day < 7) return `${day}d ago`;
  return new Date(iso).toLocaleDateString();
}
