"use client";

import {
  Bar,
  BarChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { ApiKeyUsage } from "@/lib/api";
import styles from "./detail.module.css";

/**
 * Two charts side-by-side plus a per-agent table. Uses recharts.
 *
 * Today the backend returns zeros — the relay invoke handler still has
 * to be wired to stamp `api_key_id` on every invocation row. We render
 * the empty state explicitly so the layout doesn't look broken.
 */
export function UsageCharts({ usage }: { usage: ApiKeyUsage | null }) {
  if (!usage) {
    return (
      <section className={styles.usageSection}>
        <h2 className={styles.usageTitle}>Usage</h2>
        <p className={styles.empty}>
          Couldn&apos;t load usage data. Try refreshing — the endpoint is
          live, but the relay invoke handler has not been wired to record
          per-call <code>api_key_id</code> yet, so the table is empty by
          design.
        </p>
      </section>
    );
  }

  const hasData = usage.total_requests > 0;

  return (
    <section className={styles.usageSection}>
      <header className={styles.usageHead}>
        <h2 className={styles.usageTitle}>Usage</h2>
        <p className={styles.usageMeta}>
          <strong>{usage.total_requests.toLocaleString()}</strong> request
          {usage.total_requests === 1 ? "" : "s"} ·{" "}
          {new Date(usage.from).toLocaleDateString()} →{" "}
          {new Date(usage.to).toLocaleDateString()}
        </p>
      </header>

      {!hasData && (
        <p className={styles.usageNote}>
          No requests in this window. Once the relay invoke handler stamps
          <code> api_key_id</code> on each invocation, daily counts and
          capability breakdowns appear here automatically.
        </p>
      )}

      <div className={styles.chartGrid}>
        <div className={styles.chartCard}>
          <h3 className={styles.chartTitle}>Daily requests</h3>
          <div className={styles.chartFrame}>
            <ResponsiveContainer width="100%" height={220}>
              <BarChart
                data={fillDailySeries(usage)}
                margin={{ top: 12, right: 12, left: -12, bottom: 4 }}
              >
                <XAxis
                  dataKey="date"
                  tick={{ fontSize: 11 }}
                  tickFormatter={(v: string) => v.slice(5)}
                  interval="preserveStartEnd"
                />
                <YAxis
                  allowDecimals={false}
                  tick={{ fontSize: 11 }}
                  width={32}
                />
                <Tooltip
                  cursor={{ fill: "rgba(0,0,0,0.04)" }}
                  contentStyle={tooltipStyle}
                />
                <Bar
                  dataKey="count"
                  fill="var(--accent-coral)"
                  radius={[3, 3, 0, 0]}
                />
              </BarChart>
            </ResponsiveContainer>
          </div>
        </div>

        <div className={styles.chartCard}>
          <h3 className={styles.chartTitle}>By capability</h3>
          <div className={styles.chartFrame}>
            {usage.by_capability_type.length === 0 ? (
              <p className={styles.chartEmpty}>No capability data.</p>
            ) : (
              <ResponsiveContainer width="100%" height={220}>
                <PieChart margin={{ top: 8, right: 8, bottom: 8, left: 8 }}>
                  <Tooltip contentStyle={tooltipStyle} />
                  <Pie
                    data={usage.by_capability_type}
                    dataKey="count"
                    nameKey="capability_name"
                    cx="50%"
                    cy="50%"
                    innerRadius={48}
                    outerRadius={84}
                    paddingAngle={2}
                  >
                    {usage.by_capability_type.map((_, i) => (
                      <Cell key={i} fill={pieColor(i)} />
                    ))}
                  </Pie>
                </PieChart>
              </ResponsiveContainer>
            )}
          </div>
          {usage.by_capability_type.length > 0 && (
            <ul className={styles.legendList}>
              {usage.by_capability_type.map((c, i) => (
                <li key={c.capability_name} className={styles.legendItem}>
                  <span
                    className={styles.legendSwatch}
                    style={{ background: pieColor(i) }}
                    aria-hidden
                  />
                  <code className={styles.legendCap}>{c.capability_name}</code>
                  <span className={styles.legendCount}>{c.count}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      <div className={styles.byAgentCard}>
        <h3 className={styles.chartTitle}>By agent</h3>
        {usage.by_agent.length === 0 ? (
          <p className={styles.chartEmpty}>No per-agent data.</p>
        ) : (
          <table className={styles.byAgentTable}>
            <thead>
              <tr>
                <th>Agent</th>
                <th className={styles.numericCol}>Requests</th>
              </tr>
            </thead>
            <tbody>
              {[...usage.by_agent]
                .sort((a, b) => b.count - a.count)
                .map((row) => (
                  <tr key={row.agent_id}>
                    <td>
                      <code className={styles.agentSlug}>{row.agent_slug}</code>
                    </td>
                    <td className={styles.numericCol}>
                      {row.count.toLocaleString()}
                    </td>
                  </tr>
                ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}

/**
 * Backend returns one row per day that had at least one request. For a
 * bar chart we want every day in the window so the x-axis reads as a
 * continuous timeline. Pads missing days with zero.
 */
function fillDailySeries(usage: ApiKeyUsage) {
  const map = new Map<string, number>();
  for (const d of usage.daily) {
    map.set(d.date, d.count);
  }
  const from = new Date(usage.from);
  const to = new Date(usage.to);
  // Normalise to UTC date boundaries.
  const start = Date.UTC(from.getUTCFullYear(), from.getUTCMonth(), from.getUTCDate());
  const end = Date.UTC(to.getUTCFullYear(), to.getUTCMonth(), to.getUTCDate());
  const out: { date: string; count: number }[] = [];
  for (let t = start; t <= end; t += 86_400_000) {
    const d = new Date(t);
    const iso = `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())}`;
    out.push({ date: iso, count: map.get(iso) ?? 0 });
  }
  return out;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

const PIE_PALETTE = [
  "var(--accent-coral)",
  "var(--accent-butter)",
  "oklch(75% 0.18 145)",
  "oklch(70% 0.16 250)",
  "oklch(72% 0.16 320)",
  "oklch(78% 0.12 200)",
];

function pieColor(i: number): string {
  return PIE_PALETTE[i % PIE_PALETTE.length];
}

const tooltipStyle: React.CSSProperties = {
  fontFamily: "var(--font-body)",
  fontSize: "0.82rem",
  borderRadius: "0.5rem",
  border: "1px solid var(--line)",
  background: "var(--paper-soft)",
  color: "var(--ink)",
};
