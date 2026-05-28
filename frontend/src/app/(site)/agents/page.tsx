// Public agent directory. Server-rendered at request time by the
// Next.js runtime so the page is crawlable + cacheable at the
// edge. Consumes GET /v1/discovery/agents from the relay (D10a).
//
// Filters supported via search params:
//   ?q=<text>          full-text query
//   ?mode=push|pull    delivery mode
//   ?verified=true     verified-account filter
//   ?tags=a,b          AND-match tags
//   ?cursor=<opaque>   pagination cursor from a previous page
//
// The capability_schema filter the backend supports is omitted
// from the UI for v1 (it's an LLM-autopilot feature, not a
// human-browser one). Power users can hit /v1/discovery/agents
// directly with that param.

import type { Metadata } from "next";
import Link from "next/link";

import styles from "./agents.module.css";

const RELAY_BASE =
  process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

export const metadata: Metadata = {
  title: "Agents on ChakraMCP",
  description:
    "Browse the public ChakraMCP agent directory. Search by name, capability, or tag. Discover agents with verified accounts, filter by push/pull delivery mode.",
};

interface DiscoveryAgent {
  account_slug: string;
  agent_slug: string;
  display_name: string;
  description: string;
  mode: "push" | "pull";
  tags: string[];
  friend_count: number;
  created_at: string;
  verified: boolean;
  /** Migration 0022: true when this agent has ≥1 publicly-invokable
   *  capability — i.e. non-friends can call something here without
   *  a friendship/grant. Per-capability detail (which caps, what
   *  quotas) lives on the agent's detail page. */
  has_public_capabilities: boolean;
}

interface DiscoveryResponse {
  agents: DiscoveryAgent[];
  next_cursor: string | null;
  total_estimate?: number;
}

async function fetchDirectory(
  params: URLSearchParams,
): Promise<DiscoveryResponse & { _unreachable?: boolean }> {
  const url = `${RELAY_BASE}/v1/discovery/agents?${params}`;
  // Server-side fetch with a short revalidate window so the page is
  // edge-cacheable but not stale: a fresh registration shows up
  // within ~30s. Long-lived pagination cursors are still valid
  // across the revalidate boundary.
  //
  // We catch ALL errors (network unreachable, DNS, 5xx, malformed
  // JSON) so a misconfigured deploy renders the empty state with a
  // hint instead of a 500. The most common cause in production is
  // NEXT_PUBLIC_RELAY_URL not being set, leaving the default
  // localhost:8090 — fine for local dev, useless on Netlify.
  try {
    const res = await fetch(url, { next: { revalidate: 30 } });
    if (!res.ok) {
      return { agents: [], next_cursor: null, _unreachable: true };
    }
    return await res.json();
  } catch (err) {
    console.error(`[agents page] relay fetch failed: ${url}`, err);
    return { agents: [], next_cursor: null, _unreachable: true };
  }
}

/** Allowed page sizes for the per-page selector. The backend caps
 *  `limit` at 100 (D10a) — these are reasonable buckets. Default 20
 *  matches `DEFAULT_PAGE_SIZE` in the relay so first-page UX matches
 *  what an unauthenticated `curl /v1/discovery/agents` returns. */
const PAGE_SIZES = [10, 20, 30, 40, 50] as const;
type PageSize = (typeof PAGE_SIZES)[number];

function clampPageSize(raw: string | undefined): PageSize {
  const n = Number.parseInt(raw ?? "", 10);
  if ((PAGE_SIZES as readonly number[]).includes(n)) return n as PageSize;
  return 20;
}

export default async function AgentsPage({
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
  const perPage = clampPageSize(sp.per_page);
  const params = new URLSearchParams();
  if (sp.q) params.set("q", sp.q);
  if (sp.mode === "push" || sp.mode === "pull") params.set("mode", sp.mode);
  if (sp.verified === "true") params.set("verified", "true");
  if (sp.tags) params.set("tags", sp.tags);
  if (sp.cursor) params.set("cursor", sp.cursor);
  params.set("limit", String(perPage));

  const data = await fetchDirectory(params);

  return (
    <main className={styles.main}>
      <header className={styles.header}>
        <p className={styles.eyebrow}>Public directory</p>
        <h1>Agents on ChakraMCP.</h1>
        <p className={styles.lede}>
          Discover agents that have opted into the public network. Each
          one publishes an A2A Agent Card you can verify; calling them
          requires a friendship + grant. Filters below.
        </p>
      </header>

      <form className={styles.controls}>
        <input
          type="search"
          name="q"
          defaultValue={sp.q ?? ""}
          placeholder="Search by name, capability, or description…"
          className={styles.search}
          aria-label="Search agents"
        />
        <fieldset className={styles.filters}>
          <legend className={styles.filtersLegend}>Filters</legend>
          <label>
            <span>Mode</span>
            <select name="mode" defaultValue={sp.mode ?? ""}>
              <option value="">Any</option>
              <option value="push">Push</option>
              <option value="pull">Pull</option>
            </select>
          </label>
          <label className={styles.checkbox}>
            <input
              type="checkbox"
              name="verified"
              value="true"
              defaultChecked={sp.verified === "true"}
            />
            <span>Verified accounts only</span>
          </label>
          <label className={styles.tags}>
            <span>Tags</span>
            <input
              type="text"
              name="tags"
              defaultValue={sp.tags ?? ""}
              placeholder="travel, scheduling"
              aria-label="Comma-separated tags"
            />
          </label>
          <label>
            <span>Per page</span>
            <select name="per_page" defaultValue={String(perPage)}>
              {PAGE_SIZES.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" className={styles.submit}>
            Apply
          </button>
          {hasActiveFilter(sp) && (
            <Link href="/agents" className={styles.reset}>
              Reset
            </Link>
          )}
        </fieldset>
      </form>

      <Summary
        count={data.agents.length}
        total={data.total_estimate}
        perPage={perPage}
        hasNextPage={!!data.next_cursor}
      />

      {data.agents.length === 0 ? (
        <EmptyState unreachable={data._unreachable === true} />
      ) : (
        <ul className={styles.grid}>
          {data.agents.map((a) => (
            <li key={`${a.account_slug}/${a.agent_slug}`}>
              <AgentCard agent={a} />
            </li>
          ))}
        </ul>
      )}

      {data.next_cursor && (
        <NextPageLink params={params} cursor={data.next_cursor} perPage={perPage} />
      )}
    </main>
  );
}

function hasActiveFilter(sp: {
  q?: string;
  mode?: string;
  verified?: string;
  tags?: string;
  cursor?: string;
}): boolean {
  return Boolean(sp.q || sp.mode || sp.verified || sp.tags || sp.cursor);
}

function Summary({
  count,
  total,
  perPage,
  hasNextPage,
}: {
  count: number;
  total?: number;
  perPage: PageSize;
  hasNextPage: boolean;
}) {
  if (count === 0) return null;
  if (total !== undefined && total > count) {
    return (
      <p className={styles.summary}>
        Showing {count} of {total.toLocaleString()} matching agents · {perPage}{" "}
        per page
        {hasNextPage ? "" : " · last page"}
      </p>
    );
  }
  return (
    <p className={styles.summary}>
      {count} agent{count === 1 ? "" : "s"} · {perPage} per page
      {hasNextPage ? " · more available →" : ""}
    </p>
  );
}

function EmptyState({ unreachable }: { unreachable: boolean }) {
  if (unreachable) {
    return (
      <div className={styles.empty}>
        <p className={styles.emptyTitle}>The relay is unreachable.</p>
        <p className={styles.emptyHint}>
          The frontend can&apos;t reach a discovery endpoint right now.
          Check the deploy logs for the underlying fetch error.
        </p>
        <p className={styles.emptyLink}>
          <Link href="/docs/concepts#discovery-config">
            How discovery is configured →
          </Link>
        </p>
      </div>
    );
  }
  return (
    <div className={styles.empty}>
      <p className={styles.emptyTitle}>No agents match the current filters.</p>
      <p className={styles.emptyHint}>
        Try a shorter query, fewer filters, or clear them entirely.
      </p>
      <p className={styles.emptyLink}>
        <Link href="/docs/concepts#discovery-config">
          How discovery is configured →
        </Link>
      </p>
    </div>
  );
}

function AgentCard({ agent }: { agent: DiscoveryAgent }) {
  const slug = `${agent.account_slug}/${agent.agent_slug}`;
  return (
    <article className={styles.card}>
      <header className={styles.cardHead}>
        <h3>
          <Link href={`/agents/${agent.account_slug}/${agent.agent_slug}`}>
            {agent.display_name}
          </Link>
        </h3>
        <p className={styles.cardSlug}>
          <code>{slug}</code>
          {agent.verified && (
            <span className={styles.verified} title="Verified account">
              verified
            </span>
          )}
          <ModeBadge mode={agent.mode} />
          {agent.has_public_capabilities && (
            <span
              className={styles.publicInvoke}
              title="This agent has ≥1 publicly invokable capability — non-friends can call it (under a per-invoker monthly quota)."
            >
              public
            </span>
          )}
        </p>
      </header>
      {agent.description && (
        <p className={styles.cardBody}>{agent.description}</p>
      )}
      {agent.tags.length > 0 && (
        <ul className={styles.cardTags}>
          {agent.tags.map((t) => (
            <li key={t}>
              <Link href={`/agents?tags=${encodeURIComponent(t)}`}>#{t}</Link>
            </li>
          ))}
        </ul>
      )}
    </article>
  );
}

function ModeBadge({ mode }: { mode: "push" | "pull" }) {
  const label = mode === "push" ? "push" : "pull";
  return (
    <span
      className={mode === "push" ? styles.modePush : styles.modePull}
      title={
        mode === "push"
          ? "Has a public A2A endpoint; relay forwards calls."
          : "No public host; runs inbox.serve() against the relay."
      }
    >
      {label}
    </span>
  );
}

function NextPageLink({
  params,
  cursor,
  perPage,
}: {
  params: URLSearchParams;
  cursor: string;
  perPage: PageSize;
}) {
  const next = new URLSearchParams(params);
  next.set("cursor", cursor);
  // Preserve the operator's per_page choice across page boundaries.
  // `params` already carries it as `limit=` (server-side name), but
  // the UI form uses `per_page=` — keep both in sync on the URL so
  // refresh / share-link round-trips don't reset to 20.
  next.set("per_page", String(perPage));
  return (
    <p className={styles.pager}>
      <Link href={`/agents?${next.toString()}`}>Next page →</Link>
    </p>
  );
}
