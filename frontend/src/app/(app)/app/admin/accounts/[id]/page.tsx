import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { auth } from "@/auth";
import {
  adminGetAccountCredits,
  adminListOrgs,
  ApiClientError,
  type AdminOrg,
  type CreditsView,
} from "@/lib/api";
import { CreditsPanel } from "@/components/credits/CreditsPanel";
import { AdminCreditControls } from "./AdminCreditControls";
import styles from "../../admin.module.css";

export const metadata: Metadata = {
  title: "Account credits · Admin · ChakraMCP",
};

/** /app/admin/accounts/[id] — one account's credits, and the operator's controls. */
export default async function AdminAccountCreditsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const session = await auth();
  if (!session?.user?.is_admin) redirect("/app");
  const token = session.backendToken;
  if (!token) redirect("/login");

  let view: CreditsView;
  let accounts: AdminOrg[];
  try {
    [view, accounts] = await Promise.all([
      adminGetAccountCredits(token, id),
      adminListOrgs(token),
    ]);
  } catch (err) {
    if (err instanceof ApiClientError && (err.status === 404 || err.status === 400)) {
      notFound();
    }
    throw err;
  }
  const account = accounts.find((a) => a.id === id);

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/admin">Admin</Link> · Account credits
        </div>
        <h1 className={styles.title}>{account?.display_name ?? "Account"}.</h1>
        <p className={styles.body}>
          {account && (
            <>
              <code>{account.slug}</code> ·{" "}
              {account.account_type === "individual" ? "personal" : "organization"}
              {account.owner_email && <> · owner {account.owner_email}</>}.{" "}
            </>
          )}
          Changes reach the relay within seconds, and each one is recorded in
          the history below.
        </p>
      </header>

      <section className={styles.section}>
        <header className={styles.sectionHead}>
          <h2>Manage</h2>
        </header>
        <AdminCreditControls accountId={id} view={view} />
      </section>

      <CreditsPanel view={view} />
    </div>
  );
}
