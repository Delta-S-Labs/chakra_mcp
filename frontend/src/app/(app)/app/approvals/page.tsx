import type { Metadata } from "next";
import Link from "next/link";
import { auth } from "@/auth";
import { describeTerms, timeLeft } from "@/lib/consent";
import {
  listActiveConsents,
  listApprovals,
  listMessages,
  type ApprovalSummary,
  type ConsentSummary,
  type MessageSummary,
} from "@/lib/relay";
import { redirectIfUnauthorized } from "@/lib/session-expired";
import { LocalTime } from "../audit/LocalTime";
import { RevokeButton } from "./RevokeButton";
import styles from "./approvals.module.css";

export const metadata: Metadata = {
  title: "Approvals · ChakraMCP",
};

type Tab = "waiting" | "active" | "history";

const TABS: { id: Tab; label: string }[] = [
  { id: "waiting", label: "Waiting" },
  { id: "active", label: "Active consents" },
  { id: "history", label: "History" },
];

/**
 * /app/approvals — requests to your agents that wait for your approval,
 * messages to them that wait for your answer (`human_in_loop` calls),
 * the standing consents you've granted (revocable), and the last 90 days
 * of decisions.
 */
export default async function ApprovalsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const raw = (await searchParams).tab;
  const tab: Tab = raw === "active" || raw === "history" ? raw : "waiting";
  const session = await auth();
  const token = session?.backendToken ?? null;
  if (!token) {
    return <div className={styles.error}>Sign in again - no backend token in this session.</div>;
  }

  let requests: ApprovalSummary[] = [];
  let messages: MessageSummary[] = [];
  let consents: ConsentSummary[] = [];
  let error: string | null = null;
  try {
    if (tab === "active") consents = await listActiveConsents(token);
    else if (tab === "waiting") {
      [requests, messages] = await Promise.all([
        listApprovals(token, tab),
        // Relays before 0.7 don't list messages.
        listMessages(token).catch(() => []),
      ]);
    } else requests = await listApprovals(token, tab);
  } catch (err) {
    redirectIfUnauthorized(err, `/app/approvals?tab=${tab}`);
    error = err instanceof Error ? err.message : "Couldn't load approvals.";
  }

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Approvals</div>
        <h1 className={styles.title}>Requests that ask you first.</h1>
        <p className={styles.lede}>
          Capabilities can ask their owner before a request reaches the agent. Approve one
          request at a time, or for a while, and revoke a standing consent whenever you like.
        </p>
      </header>

      <nav className={styles.tabs} aria-label="Approvals">
        {TABS.map((t) => (
          <Link
            key={t.id}
            href={t.id === "waiting" ? "/app/approvals" : `/app/approvals?tab=${t.id}`}
            className={t.id === tab ? styles.tabActive : styles.tab}
            aria-current={t.id === tab ? "page" : undefined}
          >
            {t.label}
          </Link>
        ))}
      </nav>

      {error && <p className={styles.error}>{error}</p>}

      {tab === "active" ? (
        consents.length === 0 ? (
          <p className={styles.empty}>No standing consents. Approvals for just one request don&apos;t appear here.</p>
        ) : (
          <ul className={styles.list}>
            {consents.map((c) => (
              <li key={c.id} className={styles.row}>
                <div className={styles.rowMain}>
                  <span className={styles.rowTitle}>
                    {c.requester_display_name ?? "An agent"} → {c.capability_name ?? "a deleted capability"}
                  </span>
                  <span className={styles.rowMeta}>
                    {c.requester_account_display_name && <>{c.requester_account_display_name} · </>}
                    on {c.target.display_name} · until <LocalTime iso={c.valid_until} /> ·{" "}
                    {c.max_requests ? `${c.used} of ${c.max_requests} requests used` : `${c.used} used`}
                  </span>
                  {c.purpose && <span className={styles.rowMeta}>“{c.purpose}”</span>}
                </div>
                <RevokeButton consentId={c.id} />
              </li>
            ))}
          </ul>
        )
      ) : requests.length === 0 && messages.length === 0 ? (
        <p className={styles.empty}>
          {tab === "waiting"
            ? "Nothing is waiting for you."
            : "No decisions in the last 90 days."}
        </p>
      ) : (
        <ul className={styles.list}>
          {messages.map((m) => (
            <li key={m.invocation_id}>
              <Link
                href={`/app/messages/${m.invocation_id}`}
                className={`${styles.row} ${styles.rowMain}`}
              >
                <span className={styles.rowTitle}>
                  Message from {m.requester_display_name ?? "an agent"} → {m.target.display_name ?? "your agent"}
                </span>
                <span className={styles.rowMeta}>
                  {m.requester_account_display_name && <>{m.requester_account_display_name} · </>}
                  {m.capability_name} · waiting for your answer
                  {m.expires_at ? <> · {timeLeft(m.expires_at)}</> : null}
                </span>
              </Link>
            </li>
          ))}
          {requests.map((r) => (
            <li key={r.id}>
              <Link href={`/app/approvals/${r.id}`} className={`${styles.row} ${styles.rowMain}`}>
                <span className={styles.rowTitle}>
                  {r.kind === "reply"
                    ? `Reply from ${r.target.display_name ?? "your agent"} → ${r.requester_display_name ?? "an agent"}`
                    : `${r.requester_display_name ?? "An agent"} → ${r.capability_name ?? "a deleted capability"}`}
                </span>
                <span className={styles.rowMeta}>
                  {r.requester_account_display_name && <>{r.requester_account_display_name} · </>}
                  {r.kind === "reply" ? (
                    <>{r.capability_name ?? "a deleted capability"}</>
                  ) : (
                    <>on {r.target.display_name} · {describeTerms(r.approved_terms ?? r.terms)}</>
                  )}
                  {tab === "waiting" ? <> · {timeLeft(r.expires_at)}</> : null}
                </span>
                {tab === "history" && (
                  <span className={styles.status}>
                    {r.covered ? "Covered by a consent" : r.status}
                    {r.decided_by_display_name ? ` · ${r.decided_by_display_name}` : ""}
                  </span>
                )}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
