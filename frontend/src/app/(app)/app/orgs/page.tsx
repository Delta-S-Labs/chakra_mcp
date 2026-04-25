import Link from "next/link";
import { auth } from "@/auth";
import { listOrgs } from "@/lib/api";
import { CreateOrgForm } from "./CreateOrgForm";
import styles from "./orgs.module.css";

export default async function OrgsPage() {
  const session = await auth();
  const token = session?.backendToken;

  let orgs: Awaited<ReturnType<typeof listOrgs>> = [];
  let backendError: string | null = null;
  if (token) {
    try {
      orgs = await listOrgs(token);
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Backend unavailable.";
    }
  }

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
          <ul className={styles.list}>
            {personal.map((o) => (
              <li key={o.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>{o.display_name}</div>
                  <div className={styles.rowMeta}>
                    <code>{o.slug}</code> · {o.role}
                  </div>
                </div>
              </li>
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
          <ul className={styles.list}>
            {teams.map((o) => (
              <li key={o.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>{o.display_name}</div>
                  <div className={styles.rowMeta}>
                    <code>{o.slug}</code> · {o.role}
                  </div>
                </div>
                <Link className={styles.openLink} href={`/app/orgs/${o.slug}`}>
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
