import type { Metadata } from "next";
import Link from "next/link";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getOrgCredits, listOrgs, type CreditsView, type Org } from "@/lib/api";
import { BuyCredits } from "@/components/credits/BuyCredits";
import { CreditsPanel } from "@/components/credits/CreditsPanel";
import { formatCredits } from "@/lib/format";
import styles from "./credits.module.css";

export const metadata: Metadata = {
  title: "Credits · ChakraMCP",
  description:
    "Your accounts' credit balance, monthly free grant, rate limit and spend on the ChakraMCP relay.",
};

/**
 * /app/credits — one account's credits at a time: the personal account
 * by default, any other with `?account=<slug>`. Every member can see an
 * account's credits and, on chakramcp.com, buy more; only the operator can
 * grant or adjust them (/app/admin). Dodo sends a buyer back here with
 * `?checkout=<id>` once they've paid.
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
  const returnedCheckout =
    typeof params.checkout === "string" && /^[0-9a-f-]{36}$/i.test(params.checkout)
      ? params.checkout
      : undefined;
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
        <p className={styles.body}>{intro(view)}</p>
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
      {view && current && (
        <CreditsPanel
          view={view}
          buy={
            view.purchase ? (
              <BuyCredits
                key={current.slug}
                slug={current.slug}
                purchase={view.purchase}
                costPerInvocationMc={view.cost_per_invocation_mc}
                returnedCheckoutId={returnedCheckout}
              />
            ) : null
          }
        />
      )}
    </div>
  );
}

function intro(view: CreditsView | null): string {
  if (!view) return "Every accepted invocation spends a fraction of a credit.";
  if (!view.enabled) return "Credits are off on this server.";
  return [
    `Each accepted call costs ${formatCredits(view.cost_per_invocation_mc)} credit.`,
    view.purchase
      ? `$1 buys ${view.purchase.credits_per_usd.toLocaleString("en-US")} credits.`
      : null,
    `This account gets ${formatCredits(view.monthly_free_grant_mc)} free credits on the 1st of every month, and unused credits roll over.`,
  ]
    .filter(Boolean)
    .join(" ");
}
