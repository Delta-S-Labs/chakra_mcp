import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getMe } from "@/lib/api";
import styles from "./dashboard.module.css";

/**
 * /app — relay app dashboard.
 *
 * Reads the session from NextAuth, then pulls the canonical user +
 * memberships from the backend `/v1/me` endpoint. If the backend reports
 * `survey_required: true`, hard-redirects to /app/welcome.
 */
export default async function AppDashboard() {
  const session = await auth();
  const token = session?.backendToken;
  const name = session?.user?.name;

  let memberships: Awaited<ReturnType<typeof getMe>>["memberships"] = [];
  let surveyRequired = false;
  let backendError: string | null = null;
  if (token) {
    try {
      const me = await getMe(token);
      surveyRequired = me.survey_required;
      memberships = me.memberships;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Backend unavailable.";
    }
  } else {
    backendError = "No backend token in session — sign in again.";
  }

  // redirect() throws NEXT_REDIRECT; keep it outside the try/catch above so it
  // doesn't get swallowed as a backend error.
  if (surveyRequired) {
    redirect("/app/welcome");
  }

  const personal = memberships.find((m) => m.account_type === "individual");
  const orgs = memberships.filter((m) => m.account_type === "organization");

  return (
    <>
      <section className={styles.welcome}>
        <div className={styles.eyebrow}>Dashboard</div>
        <h1 className={styles.title}>You&apos;re in.</h1>
        <p className={styles.body}>
          Hi {name?.split(" ")[0] ?? "there"}. You&apos;re signed in to the relay
          app.{session?.user?.is_admin ? " Admin tab is visible — you can review users, orgs, and API keys." : ""}
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
              <p>No accounts yet — try signing out and back in to bootstrap.</p>
            </article>
          )}
        </div>
      </section>

      <section className={styles.placeholders}>
        <article className={styles.tile}>
          <div className={styles.tileEyebrow}>Soon</div>
          <h2>Agents</h2>
          <p>Register agents you own. Publish capabilities. Toggle visibility.</p>
          <span className={styles.tileBadge}>Pending Phase 1.5</span>
        </article>
        <article className={styles.tile}>
          <div className={styles.tileEyebrow}>Soon</div>
          <h2>Friendships</h2>
          <p>Inbound and outbound proposals. Counteroffer, accept, reject.</p>
          <span className={styles.tileBadge}>Pending Phase 1.5</span>
        </article>
        <article className={styles.tile}>
          <div className={styles.tileEyebrow}>Soon</div>
          <h2>Grants &amp; consent</h2>
          <p>Active directional grants, consent records, revocation history.</p>
          <span className={styles.tileBadge}>Pending Phase 1.5</span>
        </article>
        <article className={styles.tile}>
          <div className={styles.tileEyebrow}>Soon</div>
          <h2>Audit log</h2>
          <p>Every relay call, every actor context, every outcome.</p>
          <span className={styles.tileBadge}>Pending Phase 1.5</span>
        </article>
      </section>
    </>
  );
}
