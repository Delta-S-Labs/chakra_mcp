import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { auth } from "@/auth";
import { getOrg, getOrgSettings } from "@/lib/api";
import { SettingsForm } from "./SettingsForm";
import styles from "../../orgs.module.css";

/**
 * /app/orgs/[slug]/settings — owner|admin-only panel for the two
 * org-level toggles introduced in PR-H:
 *
 *   • default_agent_visibility — pre-fills the visibility dropdown
 *     when someone creates an agent under this account.
 *   • auto_friendship_enabled — placeholder in PR-H (stored only;
 *     enforcement lands in PR-I). Saving it here doesn't yet
 *     backfill the friendships table.
 *
 * Members (non-owner/admin) get redirected back to the org page so
 * they don't see a 403-shaped permission error inline.
 */
export default async function OrgSettingsPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const session = await auth();
  const token = session?.backendToken;
  if (!token) notFound();

  let org: Awaited<ReturnType<typeof getOrg>>;
  let settings: Awaited<ReturnType<typeof getOrgSettings>>;
  try {
    [org, settings] = await Promise.all([
      getOrg(token, slug),
      getOrgSettings(token, slug),
    ]);
  } catch {
    notFound();
  }

  if (org.account_type !== "organization") {
    // Personal accounts don't expose org-level settings.
    redirect(`/app/orgs/${slug}`);
  }

  if (org.role !== "owner" && org.role !== "admin") {
    redirect(`/app/orgs/${slug}`);
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href={`/app/orgs/${slug}`} className={styles.backLink}>
            ← {org.display_name}
          </Link>
        </div>
        <h1 className={styles.title}>Org settings</h1>
        <p className={styles.body}>
          Knobs that apply to every agent + member under{" "}
          <code>{org.slug}</code>. Only owners and admins see this page.
        </p>
      </header>

      <SettingsForm slug={slug} initial={settings} />
    </div>
  );
}
