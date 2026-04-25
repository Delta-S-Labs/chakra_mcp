import { notFound } from "next/navigation";
import { auth } from "@/auth";
import { getOrg, listMembers } from "@/lib/api";
import { InviteForm } from "./InviteForm";
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
    </div>
  );
}
