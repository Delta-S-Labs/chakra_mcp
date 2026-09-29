import type {
  CreditStatus,
  CreditsLedgerEntry,
  CreditsSettingChange,
  CreditsView,
} from "@/lib/api";
import { formatCredits } from "@/lib/format";
import styles from "./credits.module.css";

/**
 * An account's credits: balance, what they cost, the monthly grant and
 * limits, the last 30 days of spend, and the ledger. Rendered as-is for
 * members (/app/credits) and for the operator (/app/admin/accounts/[id]),
 * where each ledger entry also names the admin who made it.
 */
export function CreditsPanel({ view }: { view: CreditsView }) {
  const maxDaily = Math.max(1, ...view.daily.map((d) => d.spent_mc));

  return (
    <div className={styles.panel}>
      <div className={styles.summary}>
        <div className={styles.balanceCard}>
          <div className={styles.label}>Balance</div>
          <div className={styles.balance}>
            {formatCredits(view.balance_mc)}
            <span className={styles.unit}>credits</span>
          </div>
          <StatusPill status={view.status} />
          {view.status === "blocked" && (
            <p className={styles.warn}>
              Out of credits: invocations are refused until the next free
              grant on {formatDay(view.next_grant_on)}, or until credits are
              added.
            </p>
          )}
          {!view.has_wallet && (
            <p className={styles.hint}>
              This month&apos;s free grant is added with the first invocation.
            </p>
          )}
        </div>

        <dl className={styles.facts}>
          <div>
            <dt>Spent this month</dt>
            <dd>
              {formatCredits(view.spent_this_month_mc)} credits
              <span className={styles.sub}>
                {view.invocations_this_month.toLocaleString("en-US")} invocations
              </span>
            </dd>
          </div>
          <div>
            <dt>Price</dt>
            <dd>
              {formatCredits(view.cost_per_invocation_mc)} credit
              <span className={styles.sub}>per accepted invocation</span>
            </dd>
          </div>
          <div>
            <dt>Monthly free grant</dt>
            <dd>
              {formatCredits(view.monthly_free_grant_mc)} credits
              <span className={styles.sub}>
                {view.monthly_free_grant_override_mc !== null && "custom · "}
                next on {formatDay(view.next_grant_on)} · unused credits roll over
              </span>
            </dd>
          </div>
          <div>
            <dt>Rate limit</dt>
            <dd>
              {view.rate_limit_per_min.toLocaleString("en-US")} / minute
              {view.rate_limit_override_per_min !== null && (
                <span className={styles.sub}>custom</span>
              )}
            </dd>
          </div>
        </dl>
      </div>

      <section className={styles.block}>
        <h3 className={styles.blockTitle}>Last 30 days</h3>
        {view.daily.length === 0 ? (
          <p className={styles.empty}>No invocations in the last 30 days.</p>
        ) : (
          <ol className={styles.daily}>
            {view.daily.map((d) => (
              <li key={d.day} className={styles.dayRow}>
                <span className={styles.dayLabel}>{formatDay(d.day)}</span>
                <span className={styles.barTrack} aria-hidden="true">
                  <span
                    className={styles.bar}
                    style={{ width: `${Math.max(2, (d.spent_mc / maxDaily) * 100)}%` }}
                  />
                </span>
                <span className={styles.dayValue}>
                  {formatCredits(d.spent_mc)} credits · {d.invocations.toLocaleString("en-US")}
                </span>
              </li>
            ))}
          </ol>
        )}
      </section>

      <section className={styles.block}>
        <h3 className={styles.blockTitle}>History</h3>
        {view.ledger.length === 0 ? (
          <p className={styles.empty}>No grants or changes yet.</p>
        ) : (
          <div className="tableScroll">
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>When</th>
                  <th>What</th>
                  <th className={styles.num}>Change</th>
                  <th className={styles.num}>Balance after</th>
                </tr>
              </thead>
              <tbody>
                {view.ledger.map((entry) => (
                  <tr key={entry.id}>
                    <td className={styles.when}>{formatWhen(entry.created_at)}</td>
                    <td>
                      <div>{describe(entry)}</div>
                      {entry.note && <div className={styles.note}>{entry.note}</div>}
                      {entry.by && <div className={styles.by}>by {entry.by}</div>}
                    </td>
                    <td className={`${styles.num} ${entry.delta_mc < 0 ? styles.minus : ""}`}>
                      {entry.delta_mc === 0
                        ? "—"
                        : `${entry.delta_mc > 0 ? "+" : ""}${formatCredits(entry.delta_mc)}`}
                    </td>
                    <td className={styles.num}>{formatCredits(entry.balance_after_mc)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <p className={styles.footnote}>
          Invocations are charged within seconds and aren&apos;t listed here;
          see the daily spend above.
        </p>
      </section>
    </div>
  );
}

export function StatusPill({ status }: { status: CreditStatus }) {
  const label = { active: "active", blocked: "out of credits", unlimited: "unlimited" }[status];
  const cls = {
    active: styles.pillOk,
    blocked: styles.pillCoral,
    unlimited: styles.pillButter,
  }[status];
  return <span className={cls}>{label}</span>;
}

function describe(entry: CreditsLedgerEntry): string {
  switch (entry.kind) {
    case "free_grant":
      return entry.period ? `Monthly free grant (${formatMonth(entry.period)})` : "Monthly free grant";
    case "grant":
      return "Credits added";
    case "adjustment":
      return "Balance adjusted";
    case "purchase":
      return "Credits purchased";
    case "settings":
      return `Settings changed: ${Object.entries(entry.changes ?? {})
        .map(([key, change]) => describeChange(key, change))
        .join("; ")}`;
  }
}

function describeChange(key: string, { from, to }: CreditsSettingChange): string {
  switch (key) {
    case "monthly_free_grant_mc":
      return `monthly grant ${grantText(from)} → ${grantText(to)}`;
    case "rate_limit_per_min":
      return `rate limit ${rateText(from)} → ${rateText(to)}`;
    case "unlimited":
      return to ? "unlimited on" : "unlimited off";
    default:
      return key;
  }
}

function grantText(value: number | boolean | null): string {
  return typeof value === "number" ? `${formatCredits(value)} credits` : "default";
}

function rateText(value: number | boolean | null): string {
  return typeof value === "number" ? `${value}/min` : "default";
}

// Dates render on the server, so pin the zone: the grant calendar is UTC.
function formatDay(isoDate: string): string {
  return new Date(`${isoDate}T00:00:00Z`).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

function formatMonth(isoDate: string): string {
  return new Date(`${isoDate}T00:00:00Z`).toLocaleDateString("en-US", {
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
}

function formatWhen(iso: string): string {
  return `${new Date(iso).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "UTC",
  })} UTC`;
}
