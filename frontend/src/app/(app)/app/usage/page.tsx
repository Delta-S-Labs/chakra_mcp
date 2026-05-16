import type { Metadata } from "next";
import { auth } from "@/auth";
import { getUsageSummary, type UsageSummary } from "@/lib/api";
import { computeRange, readRangeParam, readScopeParam } from "./range";
import { UsageView } from "./UsageView";
import styles from "./usage.module.css";

export const metadata: Metadata = {
  title: "Usage · ChakraMCP",
  description:
    "Per-org, per-agent, per-API-key and per-pair usage breakdown of your ChakraMCP relay traffic.",
};

/**
 * /app/usage — single roll-up page for everything the user can see on
 * the relay.
 *
 * Server side: fetch /v1/usage/summary once. The endpoint already
 * returns total + by_org / by_agent / by_api_key / by_pair + a daily
 * sparkline series, so the client component just slices it.
 *
 * Range selection (7d / 30d / 90d) lives on the client; switching
 * windows re-fetches /v1/usage/summary through the same helper.
 */
export default async function UsagePage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const rangeKey = readRangeParam(params);
  const scopeKey = readScopeParam(params);
  const session = await auth();
  const token = session?.backendToken;

  let initial: UsageSummary | null = null;
  let backendError: string | null = null;

  if (token) {
    try {
      initial = await getUsageSummary(token, {
        ...computeRange(rangeKey),
        scope: scopeKey,
      });
    } catch (err) {
      backendError =
        err instanceof Error ? err.message : "Couldn't load usage data.";
    }
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Usage</div>
        <h1 className={styles.title}>Where your relay traffic goes.</h1>
        <p className={styles.body}>
          Roll-ups across every credential pathway (API keys, paired agents,
          OAuth sessions) and every org you&apos;re a member of. Defaults to the
          last 30 days; use the buttons to widen or tighten the window.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      {token && initial && (
        <UsageView
          initial={initial}
          initialRange={rangeKey}
          initialScope={scopeKey}
          token={token}
        />
      )}

      {!token && (
        <div className={styles.error}>
          You aren&apos;t signed in. Try refreshing.
        </div>
      )}
    </div>
  );
}

