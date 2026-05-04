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
}

interface DiscoveryResponse {
  agents: DiscoveryAgent[];
  next_cursor: string | null;
  total_estimate?: number;
}

async function fetchDirectory(params: URLSearchParams): Promise<DiscoveryResponse> {
  const url = `${RELAY_BASE}/v1/discovery/agents?${params}`;
  // Server-side fetch with a short revalidate window so the page is
  // edge-cacheable but not stale: a fresh registration shows up
  // within ~30s. Long-lived pagination cursors are still valid
  // across the revalidate boundary.
  const res = await fetch(url, { next: { revalidate: 30 } });
  if (!res.ok) {
    return { agents: [], next_cursor: null };
  }
  return res.json();
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
  }>;
}) {
  const sp = await searchParams;
  const params = new URLSearchParams();
  if (sp.q) params.set("q", sp.q);
  if (sp.mode === "push" || sp.mode === "pull") params.set("mode", sp.mode);
  if (sp.verified === "true") params.set("verified", "true");
  if (sp.tags) params.set("tags", sp.tags);
  if (sp.cursor) params.set("cursor", sp.cursor);

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

      <Summary count={data.agents.length} total={data.total_estimate} />

      {data.agents.length === 0 ? (
        <EmptyState />
      ) : (
        <ul className={styles.grid}>
          {data.agents.map((a) => (
            <li key={`${a.account_slug}/${a.agent_slug}`}>
              <AgentCard agent={a} />
            </li>
          ))}
        </ul>
      )}

      {data.next_cursor && <NextPageLink params={params} cursor={data.next_cursor} />}
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

function Summary({ count, total }: { count: number; total?: number }) {
  if (count === 0) return null;
  if (total !== undefined && total > count) {
    return (
      <p className={styles.summary}>
        Showing {count} of {total.toLocaleString()} matching agents.
      </p>
    );
  }
  return <p className={styles.summary}>{count} agents.</p>;
}

function EmptyState() {
  return (
    <div className={styles.empty}>
      <p>No agents match the current filters.</p>
      <p className={styles.emptyHint}>
        Try clearing the search box, or check whether{" "}
        <code>DISCOVERY_V2</code> is enabled on your relay.
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
}: {
  params: URLSearchParams;
  cursor: string;
}) {
  const next = new URLSearchParams(params);
  next.set("cursor", cursor);
  return (
    <p className={styles.pager}>
      <Link href={`/agents?${next.toString()}`}>Next page →</Link>
    </p>
  );
}
