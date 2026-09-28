import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getOrgCredits, listOrgs, type CreditsView, type Org } from "@/lib/api";
import { CreditsPanel } from "@/components/credits/CreditsPanel";
import styles from "./credits.module.css";

export const metadata: Metadata = {
  title: "Credits · ChakraMCP",
  description:
    "Your accounts' credit balance, monthly free grant, rate limit and spend on the ChakraMCP relay.",
};

/**
 * /app/credits — one account's credits at a time: the personal account
 * by default, any other with `?account=<slug>`. Every member can see an
 * account's credits; only the operator can change them (/app/admin).
 */
export default async function CreditsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const session = await auth();
  const token = session?.backendToken;
  if (!token) redirect("/login");

  let accounts: Org[] = [];
  let view: CreditsView | null = null;
  let error: string | null = null;
  try {
    accounts = await listOrgs(token);
  } catch (err) {
    error = err instanceof Error ? err.message : "Couldn't load your accounts.";
  }

  const wanted = typeof params.account === "string" ? params.account : undefined;
  const current =
    accounts.find((a) => a.slug === wanted) ??
    accounts.find((a) => a.account_type === "individual") ??
    accounts[0];
  if (current) {
    try {
      view = await getOrgCredits(token, current.slug);
    } catch (err) {
      error = err instanceof Error ? err.message : "Couldn't load credits.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Credits</div>
        <h1 className={styles.title}>Credits.</h1>
        <p className={styles.body}>
          Every accepted invocation spends a fraction of a credit. Each account
          gets free credits on the 1st of every month, and whatever you
          don&apos;t use rolls over. Buying more credits is coming soon.
        </p>
      </header>

      {accounts.length > 1 && (
        <nav className={styles.switcher} aria-label="Account">
          {accounts.map((a) => (
            <Link
              key={a.id}
              href={`/app/credits?account=${encodeURIComponent(a.slug)}`}
              className={a.id === current?.id ? styles.tabActive : styles.tab}
              aria-current={a.id === current?.id ? "page" : undefined}
            >
              {a.account_type === "individual" ? "Personal" : a.display_name}
            </Link>
          ))}
        </nav>
      )}

      {error && <div className={styles.error}>{error}</div>}
      {view && <CreditsPanel view={view} />}
    </div>
  );
}
