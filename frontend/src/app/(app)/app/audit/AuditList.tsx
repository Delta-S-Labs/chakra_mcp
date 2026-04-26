"use client";

import { useMemo, useState } from "react";
import type { Invocation, InvocationStatus } from "@/lib/relay";
import styles from "./audit.module.css";

const STATUSES: InvocationStatus[] = [
  "pending",
  "in_progress",
  "succeeded",
  "failed",
  "timeout",
  "rejected",
];
const DIRECTIONS = [
  { id: "all", label: "All" },
  { id: "outbound", label: "Outbound" },
  { id: "inbound", label: "Inbound" },
] as const;

export function AuditList({ items }: { items: Invocation[] }) {
  const [direction, setDirection] = useState<"all" | "outbound" | "inbound">("all");
  const [statusFilter, setStatusFilter] = useState<InvocationStatus | "all">("all");
  const [openId, setOpenId] = useState<string | null>(null);

  const filtered = useMemo(
    () =>
      items.filter((i) => {
        if (direction === "outbound" && !i.i_served) return false;
        if (direction === "inbound" && !i.i_invoked) return false;
        if (statusFilter !== "all" && i.status !== statusFilter) return false;
        return true;
      }),
    [items, direction, statusFilter],
  );

  if (items.length === 0) {
    return (
      <p className={styles.empty}>
        No invocations yet. Once a friend invokes one of your capabilities (or
        you invoke one of theirs from <strong>Grants</strong>), you&apos;ll see
        the trail here.
      </p>
    );
  }

  return (
    <>
      <div className={styles.filters}>
        <div className={styles.tabs}>
          {DIRECTIONS.map((d) => (
            <button
              key={d.id}
              type="button"
              className={`${styles.tab} ${direction === d.id ? styles.tabOn : ""}`}
              onClick={() => setDirection(d.id)}
            >
              {d.label}
            </button>
          ))}
        </div>
        <div className={styles.tabs}>
          <button
            type="button"
            className={`${styles.tab} ${statusFilter === "all" ? styles.tabOn : ""}`}
            onClick={() => setStatusFilter("all")}
          >
            All statuses
          </button>
          {STATUSES.map((s) => (
            <button
              key={s}
              type="button"
              className={`${styles.tab} ${statusFilter === s ? styles.tabOn : ""}`}
              onClick={() => setStatusFilter(s)}
            >
              {s}
            </button>
          ))}
        </div>
      </div>

      {filtered.length === 0 ? (
        <p className={styles.empty}>Nothing matches that filter.</p>
      ) : (
        <ul className={styles.list}>
          {filtered.map((i) => (
            <Row
              key={i.id}
              item={i}
              expanded={openId === i.id}
              onToggle={() => setOpenId((cur) => (cur === i.id ? null : i.id))}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function Row({
  item,
  expanded,
  onToggle,
}: {
  item: Invocation;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <li className={styles.row}>
      <button type="button" className={styles.rowHeader} onClick={onToggle}>
        <div className={styles.rowName}>
          <strong>{item.granter_display_name ?? "deleted agent"}</strong>
          <code className={styles.capCode}>{item.capability_name}</code>
          <span className={styles.arrow}>←</span>{" "}
          <strong>{item.grantee_display_name ?? "deleted agent"}</strong>
          <StatusBadge status={item.status} />
        </div>
        <div className={styles.rowMeta}>
          <span>{new Date(item.created_at).toLocaleString()}</span>
          {item.elapsed_ms > 0 && (
            <>
              <span>·</span>
              <span>{item.elapsed_ms}ms</span>
            </>
          )}
          {item.claimed_at && item.status === "in_progress" && (
            <>
              <span>·</span>
              <span>claimed {new Date(item.claimed_at).toLocaleTimeString()}</span>
            </>
          )}
          {item.error_message && (
            <>
              <span>·</span>
              <span className={styles.errorText}>{item.error_message}</span>
            </>
          )}
        </div>
      </button>

      {expanded && (
        <div className={styles.detail}>
          <Section title="Input">
            <Pre value={item.input_preview} />
          </Section>
          <Section title="Output">
            <Pre value={item.output_preview} />
          </Section>
        </div>
      )}
    </li>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className={styles.section}>
      <div className={styles.sectionTitle}>{title}</div>
      {children}
    </div>
  );
}

function Pre({ value }: { value: unknown }) {
  if (value == null) {
    return <p className={styles.empty}>—</p>;
  }
  return <pre className={styles.pre}>{JSON.stringify(value, null, 2)}</pre>;
}

function StatusBadge({ status }: { status: InvocationStatus }) {
  const cls =
    status === "succeeded"
      ? styles.badgeOk
      : status === "in_progress" || status === "pending"
      ? styles.badgeWarn
      : status === "rejected"
      ? styles.badgeWarn
      : styles.badgeBad;
  return <span className={`${styles.badge} ${cls}`}>{status}</span>;
}
