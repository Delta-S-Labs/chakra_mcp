"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import type { Agent } from "@/lib/relay";
import styles from "../agents.module.css";

const PAGE_SIZE = 20;

/**
 * Client-side search + pagination over the "others on the network"
 * list. The relay's `/v1/network/agents` returns the full list in one
 * shot (no server pagination yet) — at hundreds of agents this is
 * fine; if it grows past low-thousands we'll need to push pagination
 * into the backend. The filter searches display_name, the owning
 * account's display_name, and the slug, all case-insensitive.
 *
 * Why client-side over Suspense + server actions:
 *   - The full payload is already paid for (server component fetches
 *     it once and passes in).
 *   - Filter latency = synchronous slice, no round-trip.
 *   - Pagination state is purely UI — no need to put it in the URL.
 */
export function NetworkOthersList({ others }: { others: Agent[] }) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return others;
    return others.filter((a) => {
      const hay = `${a.display_name} ${a.account_display_name ?? ""} ${a.slug}`.toLowerCase();
      return hay.includes(q);
    });
  }, [others, query]);

  // Reset to page 0 whenever the filter changes so the user doesn't
  // land on an empty page 6 after typing a query that only matches 3.
  const pageStart = Math.min(page, Math.max(0, Math.ceil(filtered.length / PAGE_SIZE) - 1)) * PAGE_SIZE;
  const visible = filtered.slice(pageStart, pageStart + PAGE_SIZE);
  const totalPages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.floor(pageStart / PAGE_SIZE);

  return (
    <>
      <div className={styles.toolbar}>
        <input
          type="search"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setPage(0);
          }}
          placeholder="Search by name, account, or slug…"
          className={styles.searchInput}
          aria-label="Search network agents"
        />
        <span className={styles.toolbarCount}>
          {filtered.length === others.length
            ? `${others.length} ${others.length === 1 ? "agent" : "agents"}`
            : `${filtered.length} of ${others.length}`}
        </span>
      </div>

      {filtered.length === 0 ? (
        <EmptyOthers hasQuery={query.trim().length > 0} query={query.trim()} />
      ) : (
        <>
          <ul className={styles.list}>
            {visible.map((a) => (
              <li key={a.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>{a.display_name}</div>
                  <div className={styles.rowMeta}>
                    by <strong>{a.account_display_name}</strong> ·{" "}
                    {a.capability_count}{" "}
                    {a.capability_count === 1 ? "capability" : "capabilities"}
                  </div>
                </div>
                <Link className={styles.openLink} href={`/app/agents/${a.id}`}>
                  Open →
                </Link>
              </li>
            ))}
          </ul>

          {totalPages > 1 && (
            <div className={styles.pager}>
              <button
                type="button"
                className={styles.pagerBtn}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
                disabled={currentPage === 0}
              >
                ← Prev
              </button>
              <span className={styles.pagerLabel}>
                Page {currentPage + 1} of {totalPages}
              </span>
              <button
                type="button"
                className={styles.pagerBtn}
                onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
                disabled={currentPage >= totalPages - 1}
              >
                Next →
              </button>
            </div>
          )}
        </>
      )}
    </>
  );
}

function EmptyOthers({ hasQuery, query }: { hasQuery: boolean; query: string }) {
  return (
    <div className={styles.emptyState}>
      <p className={styles.emptyStateTitle}>
        {hasQuery
          ? `No agents match “${query}”.`
          : "No other agents on the network yet."}
      </p>
      <p className={styles.emptyStateBody}>
        {hasQuery
          ? "Try a shorter or different query."
          : "Once someone flips their agent's visibility to network, it'll show up here."}
      </p>
      <p className={styles.emptyStateLink}>
        <Link href="/docs/concepts#agents">
          How visibility &amp; discovery work →
        </Link>
      </p>
    </div>
  );
}
