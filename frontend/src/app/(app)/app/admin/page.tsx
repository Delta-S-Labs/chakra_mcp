import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { adminListUsers, adminListOrgs, adminListApiKeys } from "@/lib/api";
import styles from "./admin.module.css";

export default async function AdminPage() {
  const session = await auth();
  if (!session?.user?.is_admin) redirect("/app");

  const token = session.backendToken;
  if (!token) redirect("/login");

  let users: Awaited<ReturnType<typeof adminListUsers>> = [];
  let orgs: Awaited<ReturnType<typeof adminListOrgs>> = [];
  let keys: Awaited<ReturnType<typeof adminListApiKeys>> = [];
  let backendError: string | null = null;

  try {
    [users, orgs, keys] = await Promise.all([
      adminListUsers(token),
      adminListOrgs(token),
      adminListApiKeys(token),
    ]);
  } catch (err) {
    backendError = err instanceof Error ? err.message : "Backend unavailable.";
  }

  const fmt = (s: string | null | undefined) => (s ? new Date(s).toLocaleString() : "-");

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Admin</div>
        <h1 className={styles.title}>Network operator console.</h1>
        <p className={styles.body}>
          Visible only to the user whose email matches <code>ADMIN_EMAIL</code> on
          the backend. Read-only for now - actions (suspend, transfer ownership,
          revoke) land in the next slice.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      <section className={styles.section}>
        <header className={styles.sectionHead}>
          <h2>Users</h2>
          <span className={styles.count}>{users.length}</span>
        </header>
        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Email</th>
                <th>Display name</th>
                <th>Admin</th>
                <th>Created</th>
              </tr>
            </thead>
            <tbody>
              {users.length === 0 && (
                <tr>
                  <td colSpan={4} className={styles.empty}>No users yet.</td>
                </tr>
              )}
              {users.map((u) => (
                <tr key={u.id}>
                  <td>
                    <code>{u.email}</code>
                  </td>
                  <td>{u.display_name}</td>
                  <td>
                    {u.is_admin ? (
                      <span className={styles.pillCoral}>admin</span>
                    ) : (
                      <span className={styles.pillMuted}>user</span>
                    )}
                  </td>
                  <td className={styles.muted}>{fmt(u.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className={styles.section}>
        <header className={styles.sectionHead}>
          <h2>Organizations</h2>
          <span className={styles.count}>{orgs.length}</span>
        </header>
        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Slug</th>
                <th>Display name</th>
                <th>Type</th>
                <th>Members</th>
                <th>Owner</th>
                <th>Created</th>
              </tr>
            </thead>
            <tbody>
              {orgs.length === 0 && (
                <tr>
                  <td colSpan={6} className={styles.empty}>No accounts yet.</td>
                </tr>
              )}
              {orgs.map((o) => (
                <tr key={o.id}>
                  <td>
                    <code>{o.slug}</code>
                  </td>
                  <td>{o.display_name}</td>
                  <td>
                    {o.account_type === "individual" ? (
                      <span className={styles.pillMuted}>personal</span>
                    ) : (
                      <span className={styles.pillButter}>org</span>
                    )}
                  </td>
                  <td>{o.member_count}</td>
                  <td className={styles.muted}>{o.owner_email ?? "-"}</td>
                  <td className={styles.muted}>{fmt(o.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className={styles.section}>
        <header className={styles.sectionHead}>
          <h2>API keys</h2>
          <span className={styles.count}>{keys.length}</span>
        </header>
        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th>User</th>
                <th>Name</th>
                <th>Prefix</th>
                <th>Status</th>
                <th>Last used</th>
                <th>Expires</th>
                <th>Created</th>
              </tr>
            </thead>
            <tbody>
              {keys.length === 0 && (
                <tr>
                  <td colSpan={7} className={styles.empty}>No keys issued.</td>
                </tr>
              )}
              {keys.map((k) => (
                <tr key={k.id} className={k.revoked_at ? styles.revokedRow : undefined}>
                  <td>
                    <code>{k.user_email}</code>
                  </td>
                  <td>{k.name}</td>
                  <td>
                    <code>{k.prefix}…</code>
                  </td>
                  <td>
                    {k.revoked_at ? (
                      <span className={styles.pillMuted}>revoked</span>
                    ) : k.expires_at && new Date(k.expires_at) < new Date() ? (
                      <span className={styles.pillMuted}>expired</span>
                    ) : (
                      <span className={styles.pillOk}>active</span>
                    )}
                  </td>
                  <td className={styles.muted}>{fmt(k.last_used_at)}</td>
                  <td className={styles.muted}>{fmt(k.expires_at) || "never"}</td>
                  <td className={styles.muted}>{fmt(k.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
