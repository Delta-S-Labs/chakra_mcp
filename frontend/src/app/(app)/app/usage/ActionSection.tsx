"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import type { UsageActionBreakdown } from "@/lib/api";
import type { ScopeKey } from "./range";
import styles from "./usage.module.css";

/**
 * Bottom section of /app/usage. Nine fixed rows of platform-level
 * activity (friendships, grants, agents, capabilities, plus
 * inbox-side invocation count) under an Org/Personal toggle. URL
 * state via `?scope=`; default `org`.
 *
 * Layout target (from the 2026-05-16 design doc):
 *
 *   By platform action          [ Org ▼ ]
 *   ──────────────────────────────────────
 *   Inbox invocations                    3
 *   Friendships proposed                 2
 *   ...
 *
 * Personal scope renders a one-line footnote about pre-migration
 * history; Org renders nothing (Decision 4).
 */
export function ActionSection({
  scope,
  counts,
  pending,
  onChangeScope,
}: {
  scope: ScopeKey;
  counts: UsageActionBreakdown;
  pending: boolean;
  /** Refetches the summary with the new scope. Parent (UsageView)
   *  owns the data + loading state — the toggle just emits intent. */
  onChangeScope: (next: ScopeKey) => void;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();

  function flip(next: ScopeKey) {
    if (next === scope) return;
    // Update the deep-link URL so a reload / shared link lands on the
    // right scope, then notify the parent to refetch.
    const sp = new URLSearchParams(searchParams.toString());
    if (next === "org") sp.delete("scope");
    else sp.set("scope", next);
    const qs = sp.toString();
    router.replace(`${pathname}${qs ? `?${qs}` : ""}`, { scroll: false });
    onChangeScope(next);
  }

  const rows: { label: string; value: number }[] = [
    { label: "Inbox invocations", value: counts.inbox_invocations },
    { label: "Friendships proposed", value: counts.friendships_proposed },
    { label: "Friendships accepted", value: counts.friendships_accepted },
    { label: "Friendships rejected", value: counts.friendships_rejected },
    { label: "Friendships cancelled", value: counts.friendships_cancelled },
    { label: "Grants issued", value: counts.grants_issued },
    { label: "Grants revoked", value: counts.grants_revoked },
    { label: "Agents registered", value: counts.agents_registered },
    { label: "Capabilities published", value: counts.capabilities_published },
  ];

  return (
    <section className={styles.rollupCard}>
      <div className={styles.actionHeader}>
        <h2 className={styles.rollupTitle}>By platform action</h2>
        <label
          className={styles.scopeToggle}
          // Tooltip carries the Decision 5 / Security-note caveat.
          title="Org view counts every member's activity, not just yours."
        >
          <span className={styles.scopeLabel}>Scope</span>
          <select
            value={scope}
            disabled={pending}
            onChange={(e) => flip(e.target.value as ScopeKey)}
            className={styles.scopeSelect}
            aria-label="Org or personal scope"
          >
            <option value="org">Org</option>
            <option value="personal">Personal</option>
          </select>
        </label>
      </div>

      <table className={styles.rollupTable}>
        <tbody>
          {rows.map((r) => (
            <tr key={r.label}>
              <td>{r.label}</td>
              <td className={styles.numericCol}>{r.value.toLocaleString()}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {scope === "personal" && (
        <p className={styles.actionFootnote}>
          Personal-attribution data populated from 2026-05-16 onward. Older
          activity shows only in Org view.
        </p>
      )}
    </section>
  );
}
