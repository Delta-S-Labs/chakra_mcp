"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState, useTransition } from "react";
import {
  Bar,
  BarChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { getUsageSummary, type UsageSummary } from "@/lib/api";
import { computeRange, type RangeKey } from "./range";
import styles from "./usage.module.css";

/**
 * Client island for /app/usage. Owns the date-range picker (which
 * re-fetches from /v1/usage/summary) and renders five stacked
 * sections: total + daily chart, by API key, by agent, by pair, by org.
 *
 * Visual density mirrors the per-key dashboard at
 * /app/api-keys/[id] — same chart styling, same table look. Empty
 * states call out that no traffic has been recorded yet rather than
 * leaving blank tables.
 */
export function UsageView({
  initial,
  initialRange,
  token,
}: {
  initial: UsageSummary;
  initialRange: RangeKey;
  token: string;
}) {
  const router = useRouter();
  const [range, setRange] = useState<RangeKey>(initialRange);
  const [summary, setSummary] = useState<UsageSummary>(initial);
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();

  // Sync ?range=… in the URL so deep-links remember the window.
  useEffect(() => {
    const url = new URL(window.location.href);
    if (range === "30d") url.searchParams.delete("range");
    else url.searchParams.set("range", range);
    router.replace(`${url.pathname}${url.search}`, { scroll: false });
  }, [range, router]);

  function pick(next: RangeKey) {
    if (next === range) return;
    setRange(next);
    setError(null);
    startTransition(async () => {
      try {
        const fresh = await getUsageSummary(token, computeRange(next));
        setSummary(fresh);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Couldn't refresh usage.");
      }
    });
  }

  const hasOrgs = summary.by_org.length > 1;

  return (
    <>
      <div className={styles.rangeBar} role="tablist" aria-label="Time window">
        {(["7d", "30d", "90d"] as const).map((k) => (
          <button
            key={k}
            type="button"
            role="tab"
            aria-selected={range === k}
            disabled={pending}
            onClick={() => pick(k)}
            className={`${styles.rangeBtn} ${range === k ? styles.rangeBtnActive : ""}`}
          >
            {k === "7d" ? "Last 7 days" : k === "30d" ? "Last 30 days" : "Last 90 days"}
          </button>
        ))}
        <span className={styles.rangeMeta}>
          {new Date(summary.from).toLocaleDateString()} →{" "}
          {new Date(summary.to).toLocaleDateString()}
        </span>
      </div>

      {error && <div className={styles.error}>{error}</div>}

      <section className={styles.totalCard}>
        <div className={styles.totalNumbers}>
          <div className={styles.totalBig}>
            {summary.total.requests.toLocaleString()}
          </div>
          <div className={styles.totalLabel}>
            request{summary.total.requests === 1 ? "" : "s"} ·{" "}
            <span className={styles.totalOk}>
              {summary.total.succeeded.toLocaleString()} ok
            </span>
            {" · "}
            <span className={styles.totalBad}>
              {summary.total.failed.toLocaleString()} failed
            </span>
          </div>
        </div>
        <div className={styles.chartFrame}>
          {summary.daily.length === 0 ? (
            <p className={styles.empty}>
              No relay calls yet in this window. Once an agent makes its first
              proxied call, the daily chart fills in here.
            </p>
          ) : (
            <ResponsiveContainer width="100%" height={180}>
              <BarChart
                data={fillDaily(summary)}
                margin={{ top: 8, right: 8, left: -16, bottom: 4 }}
              >
                <XAxis
                  dataKey="date"
                  tick={{ fontSize: 11 }}
                  tickFormatter={(v: string) => v.slice(5)}
                  interval="preserveStartEnd"
                />
                <YAxis allowDecimals={false} tick={{ fontSize: 11 }} width={32} />
                <Tooltip
                  cursor={{ fill: "rgba(0,0,0,0.04)" }}
                  contentStyle={tooltipStyle}
                />
                <Bar
                  dataKey="requests"
                  fill="var(--accent-coral)"
                  radius={[3, 3, 0, 0]}
                />
              </BarChart>
            </ResponsiveContainer>
          )}
        </div>
      </section>

      <Section
        title="By API key"
        empty="No API-key traffic in this window."
        rows={summary.by_api_key}
        renderRow={(row) => (
          <tr key={`key:${row.id}`}>
            <td>
              <Link href={`/app/api-keys/${row.id}`} className={styles.link}>
                {row.name}
              </Link>
            </td>
            <td className={styles.numericCol}>
              {row.requests.toLocaleString()}
            </td>
          </tr>
        )}
        headers={["Name", "Requests"]}
      />

      <Section
        title="By agent"
        empty="No per-agent traffic in this window."
        rows={summary.by_agent}
        renderRow={(row) => (
          <tr key={`agent:${row.id}`}>
            <td>
              <Link href={`/app/agents/${row.id}`} className={styles.link}>
                {row.name || row.slug}
              </Link>
              {row.name && row.slug !== row.name && (
                <span className={styles.slugTag}>
                  <code>{row.slug}</code>
                </span>
              )}
            </td>
            <td className={styles.numericCol}>
              {row.requests.toLocaleString()}
            </td>
          </tr>
        )}
        headers={["Agent", "Requests"]}
      />

      <Section
        title="By pair"
        empty="No paired-session traffic in this window."
        rows={summary.by_pair}
        renderRow={(row) => (
          <tr key={`pair:${row.kind}:${row.id}`}>
            <td>
              <span className={`${styles.kindBadge} ${badgeClass(row.kind)}`}>
                {row.kind === "device_flow" ? "device" : "oauth"}
              </span>
              <span className={styles.pairLabel}>{row.label}</span>
            </td>
            <td className={styles.numericCol}>
              {row.requests.toLocaleString()}
            </td>
          </tr>
        )}
        headers={["Pair", "Requests"]}
      />

      {hasOrgs && (
        <Section
          title="By org"
          empty="No per-org traffic in this window."
          rows={summary.by_org}
          renderRow={(row) => (
            <tr key={`org:${row.id}`}>
              <td>
                <Link href={`/app/orgs/${row.slug}`} className={styles.link}>
                  {row.display_name}
                </Link>
                <span className={styles.slugTag}>
                  <code>{row.slug}</code>
                </span>
              </td>
              <td className={styles.numericCol}>
                {row.requests.toLocaleString()}
              </td>
            </tr>
          )}
          headers={["Org", "Requests"]}
        />
      )}
    </>
  );
}

function Section<T>({
  title,
  empty,
  rows,
  renderRow,
  headers,
}: {
  title: string;
  empty: string;
  rows: T[];
  renderRow: (row: T) => React.ReactNode;
  headers: [string, string];
}) {
  return (
    <section className={styles.rollupCard}>
      <h2 className={styles.rollupTitle}>{title}</h2>
      {rows.length === 0 ? (
        <p className={styles.empty}>{empty}</p>
      ) : (
        <table className={styles.rollupTable}>
          <thead>
            <tr>
              <th>{headers[0]}</th>
              <th className={styles.numericCol}>{headers[1]}</th>
            </tr>
          </thead>
          <tbody>{rows.map(renderRow)}</tbody>
        </table>
      )}
    </section>
  );
}

function badgeClass(kind: string): string {
  switch (kind) {
    case "device_flow":
      return styles.kindBadgeDevice;
    case "oauth":
      return styles.kindBadgeOauth;
    default:
      return "";
  }
}

/**
 * Pad missing days with zeros so the bar chart reads as a continuous
 * timeline. Same approach as the per-key UsageCharts helper.
 */
function fillDaily(summary: UsageSummary) {
  const map = new Map<string, number>();
  for (const d of summary.daily) map.set(d.date, d.requests);
  const from = new Date(summary.from);
  const to = new Date(summary.to);
  const start = Date.UTC(from.getUTCFullYear(), from.getUTCMonth(), from.getUTCDate());
  const end = Date.UTC(to.getUTCFullYear(), to.getUTCMonth(), to.getUTCDate());
  const out: { date: string; requests: number }[] = [];
  for (let t = start; t <= end; t += 86_400_000) {
    const d = new Date(t);
    const iso = `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())}`;
    out.push({ date: iso, requests: map.get(iso) ?? 0 });
  }
  return out;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

const tooltipStyle: React.CSSProperties = {
  fontFamily: "var(--font-body)",
  fontSize: "0.82rem",
  borderRadius: "0.5rem",
  border: "1px solid var(--line)",
  background: "var(--paper-soft)",
  color: "var(--ink)",
};
