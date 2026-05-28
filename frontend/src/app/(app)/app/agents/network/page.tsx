import Link from "next/link";
import { auth } from "@/auth";
import { listNetworkAgents, type NetworkAgent } from "@/lib/relay";
import { StarRating } from "@/components/StarRating";
import styles from "../agents.module.css";

/**
 * /app/agents/network - discovery view of every agent on the relay
 * that has flipped its visibility to `network`. Mirrors the public
 * /agents directory's filter shape (q / mode / verified / tags +
 * cursor pagination) but layered behind auth so we can surface the
 * `is_mine` distinction + capability_count.
 *
 * URL is the single source of truth for filters + pagination — the
 * page is fully server-rendered, share-links roundtrip cleanly, and
 * the filter form submits via GET so each apply is a normal nav.
 */

const PAGE_SIZES = [10, 20, 30, 40, 50] as const;
type PageSize = (typeof PAGE_SIZES)[number];

function clampPageSize(raw: string | undefined): PageSize {
  const n = Number.parseInt(raw ?? "", 10);
  if ((PAGE_SIZES as readonly number[]).includes(n)) return n as PageSize;
  return 20;
}

export default async function NetworkAgentsPage({
  searchParams,
}: {
  searchParams: Promise<{
    q?: string;
    mode?: string;
    verified?: string;
    tags?: string;
    cursor?: string;
    per_page?: string;
  }>;
}) {
  const sp = await searchParams;
  const session = await auth();
  const token = session?.backendToken;

  const perPage = clampPageSize(sp.per_page);
  const mode = sp.mode === "push" || sp.mode === "pull" ? sp.mode : undefined;
  const verified = sp.verified === "true";

  let agents: NetworkAgent[] = [];
  let nextCursor: string | null = null;
  let backendError: string | null = null;
  if (token) {
    try {
      const res = await listNetworkAgents(token, {
        q: sp.q,
        mode,
        verified: verified || undefined,
        tags: sp.tags,
        cursor: sp.cursor,
        limit: perPage,
      });
      agents = res.agents;
      nextCursor = res.next_cursor ?? null;
    } catch (err) {
      backendError = err instanceof Error ? err.message : "Relay unavailable.";
    }
  }

  // "Yours on the network" only appears on the first page (no cursor)
  // and ignores active filters — it's a quick-scan section, not part
  // of the searchable directory. Otherwise filtering by `verified` or
  // `mode` would hide the user's own agents from their own dashboard.
  const isFirstPage = !sp.cursor;
  const hasFilter = !!(sp.q || sp.mode || sp.verified || sp.tags);
  const others = agents.filter((a) => !a.is_mine);
  const mine = isFirstPage && !hasFilter ? agents.filter((a) => a.is_mine) : [];

  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">
          <Link href="/app/agents" className={styles.backLink}>
            ← Agents
          </Link>
        </div>
        <h1 className={styles.title}>The network.</h1>
        <p className={styles.body}>
          Every network-visible agent on this relay. Friendships and grants
          gate who can actually invoke whom - for now this is just a
          directory you can browse to see what&apos;s out there.
        </p>
      </header>

      {backendError && <div className={styles.error}>{backendError}</div>}

      {mine.length > 0 && (
        <section>
          <h2 className={styles.sectionTitle}>
            Yours on the network <span className={styles.count}>{mine.length}</span>
          </h2>
          <ul className={styles.list}>
            {mine.map((a) => (
              <li key={a.id} className={styles.row}>
                <div>
                  <div className={styles.rowName}>{a.display_name}</div>
                  <div className={styles.rowMeta}>
                    <code>
                      {a.account_slug}/{a.slug}
                    </code>{" "}
                    · {a.capability_count}{" "}
                    {a.capability_count === 1 ? "capability" : "capabilities"}
                    <RatingChip
                      avg={a.avg_rating}
                      count={a.review_count}
                    />
                  </div>
                </div>
                <Link className={styles.openLink} href={`/app/agents/${a.id}`}>
                  Open →
                </Link>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <h2 className={styles.sectionTitle}>
          Others
        </h2>

        <form className={styles.netFilterForm}>
          <input
            type="search"
            name="q"
            defaultValue={sp.q ?? ""}
            placeholder="Search by name, capability, or description…"
            className={styles.searchInput}
            aria-label="Search network agents"
          />
          <div className={styles.netFilterRow}>
            <label className={styles.netFilterField}>
              <span>Mode</span>
              <select name="mode" defaultValue={sp.mode ?? ""}>
                <option value="">Any</option>
                <option value="push">Push</option>
                <option value="pull">Pull</option>
              </select>
            </label>
            <label className={styles.netFilterCheckbox}>
              <input
                type="checkbox"
                name="verified"
                value="true"
                defaultChecked={sp.verified === "true"}
              />
              <span>Verified accounts only</span>
            </label>
            <label className={styles.netFilterField}>
              <span>Tags</span>
              <input
                type="text"
                name="tags"
                defaultValue={sp.tags ?? ""}
                placeholder="travel, scheduling"
                aria-label="Comma-separated tags"
              />
            </label>
            <label className={styles.netFilterField}>
              <span>Per page</span>
              <select name="per_page" defaultValue={String(perPage)}>
                {PAGE_SIZES.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <button type="submit" className={styles.netFilterApply}>
              Apply
            </button>
            {hasFilter && (
              <Link href="/app/agents/network" className={styles.netFilterReset}>
                Reset
              </Link>
            )}
          </div>
        </form>

        {others.length === 0 ? (
          <EmptyOthers hasQuery={hasFilter} query={sp.q ?? ""} />
        ) : (
          <>
            <ul className={styles.list}>
              {others.map((a) => (
                <li key={a.id} className={styles.row}>
                  <div>
                    <div className={styles.rowName}>
                      {a.display_name}
                      {a.verified && (
                        <span className={styles.verifiedChip} title="Verified account">
                          verified
                        </span>
                      )}
                      <ModeChip mode={a.mode} />
                    </div>
                    <div className={styles.rowMeta}>
                      by <strong>{a.account_display_name}</strong> ·{" "}
                      {a.capability_count}{" "}
                      {a.capability_count === 1 ? "capability" : "capabilities"}
                      <RatingChip
                        avg={a.avg_rating}
                        count={a.review_count}
                      />
                      {a.tags.length > 0 && (
                        <>
                          {" · "}
                          {a.tags.slice(0, 4).map((t) => (
                            <code key={t} className={styles.rowTag}>
                              #{t}
                            </code>
                          ))}
                        </>
                      )}
                    </div>
                  </div>
                  <Link className={styles.openLink} href={`/app/agents/${a.id}`}>
                    Open →
                  </Link>
                </li>
              ))}
            </ul>

            {nextCursor && (
              <NextPage sp={sp} cursor={nextCursor} perPage={perPage} />
            )}
          </>
        )}
      </section>
    </div>
  );
}

function RatingChip({
  avg,
  count,
}: {
  avg: number | null;
  count: number;
}) {
  if (count === 0) return null;
  return (
    <span style={{ marginLeft: "0.5rem", whiteSpace: "nowrap" }}>
      · <StarRating value={avg} size={12} /> {avg?.toFixed(1)}{" "}
      <span style={{ color: "var(--ink-muted)" }}>
        ({count})
      </span>
    </span>
  );
}

function ModeChip({ mode }: { mode: "push" | "pull" }) {
  return (
    <span
      className={mode === "push" ? styles.modePushChip : styles.modePullChip}
      title={
        mode === "push"
          ? "Has a public A2A endpoint; relay forwards calls."
          : "No public host; runs inbox.serve() against the relay."
      }
    >
      {mode}
    </span>
  );
}

function EmptyOthers({ hasQuery, query }: { hasQuery: boolean; query: string }) {
  return (
    <div className={styles.emptyState}>
      <p className={styles.emptyStateTitle}>
        {hasQuery
          ? query
            ? `No agents match “${query}”.`
            : "No agents match the current filters."
          : "No other agents on the network yet."}
      </p>
      <p className={styles.emptyStateBody}>
        {hasQuery
          ? "Try a shorter query, fewer filters, or clear them entirely."
          : "Once someone flips their agent's visibility to network, it'll show up here."}
      </p>
      <p className={styles.emptyStateLink}>
        <Link href="/docs/concepts#discovery-config">
          How visibility &amp; discovery work →
        </Link>
      </p>
    </div>
  );
}

function NextPage({
  sp,
  cursor,
  perPage,
}: {
  sp: { q?: string; mode?: string; verified?: string; tags?: string };
  cursor: string;
  perPage: PageSize;
}) {
  const next = new URLSearchParams();
  if (sp.q) next.set("q", sp.q);
  if (sp.mode) next.set("mode", sp.mode);
  if (sp.verified) next.set("verified", sp.verified);
  if (sp.tags) next.set("tags", sp.tags);
  next.set("cursor", cursor);
  next.set("per_page", String(perPage));
  return (
    <p className={styles.netPager}>
      <Link href={`/app/agents/network?${next.toString()}`}>Next page →</Link>
    </p>
  );
}
