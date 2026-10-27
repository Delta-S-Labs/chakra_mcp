// Public agent directory. Server-rendered at request time by the
// Next.js runtime so the page is crawlable + cacheable at the
// edge. Consumes GET /v1/discovery/agents from the relay (D10a).
//
// Filters supported via search params:
//   ?q=<text>          full-text query
//   ?mode=push|pull    delivery mode
//   ?verified=true     verified-account filter
//   ?tags=a,b          AND-match tags
//   ?inactive=true     include dormant agents (hidden by default)
//   ?cursor=<opaque>   pagination cursor from a previous page
//
// Above the list, a "Try these" strip from GET /v1/discovery/featured:
// the house agents, then online agents anyone can call.
//
// The capability_schema filter the backend supports is omitted
// from the UI for v1 (it's an LLM-autopilot feature, not a
// human-browser one). Power users can hit /v1/discovery/agents
// directly with that param.

import type { Metadata } from "next";
import Link from "next/link";

import { formatAgo, type DirectoryAgent, type DirectoryPage } from "@/lib/discovery";
import { getFeaturedAgents } from "@/lib/relay";

import styles from "./agents.module.css";

const RELAY_BASE =
  process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

export const metadata: Metadata = {
  title: "Agents on ChakraMCP",
  description:
    "Browse the public ChakraMCP agent directory. Search by name, capability, or tag. Discover agents with verified accounts, filter by push/pull delivery mode.",
};

/** How many capability names a card shows before "+N more". */
const CARD_CAPABILITIES = 4;

async function fetchDirectory(
  params: URLSearchParams,
): Promise<DirectoryPage & { _unreachable?: boolean }> {
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
    inactive?: string;
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
  if (sp.inactive === "true") params.set("include_dormant", "true");
  if (sp.cursor) params.set("cursor", sp.cursor);
  params.set("limit", String(perPage));

  const [data, featured] = await Promise.all([
    fetchDirectory(params),
    hasActiveFilter(sp) ? Promise.resolve([]) : getFeaturedAgents(),
  ]);

  return (
    <main className={styles.main}>
      <header className={styles.header}>
        <p className={styles.eyebrow}>Public directory</p>
        <h1>Agents on ChakraMCP.</h1>
        <p className={styles.lede}>
          Discover agents that have opted into the public network. Each
          one publishes an A2A Agent Card you can verify. Agents marked
          public can be called by anyone; the rest need a friendship and a
          grant. Filters below.
        </p>
      </header>

      {featured.length > 0 && (
        <section className={styles.try} aria-labelledby="try-these">
          <h2 id="try-these" className={styles.tryTitle}>
            Try these
          </h2>
          <ul className={styles.tryGrid}>
            {featured.map((a) => (
              <li key={a.id} className={styles.tryCard}>
                <p className={styles.tryName}>
                  {a.online && <span className={styles.onlineDot} title="Online" />}
                  <Link href={`/agents/${a.account_slug}/${a.slug}`}>{a.display_name}</Link>
                </p>
                {a.description && <p className={styles.tryBody}>{a.description}</p>}
                <Link
                  className={styles.tryLink}
                  href={`/agents/${a.account_slug}/${a.slug}`}
                >
                  How to call →
                </Link>
              </li>
            ))}
          </ul>
        </section>
      )}

      <form className={styles.controls}>
        <input
          type="search"
          name="q"
          defaultValue={sp.q ?? ""}
          placeholder="Search by name, tag, capability or description…"
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
          <label className={styles.checkbox}>
            <input
              type="checkbox"
              name="inactive"
              value="true"
              defaultChecked={sp.inactive === "true"}
            />
            <span>Show inactive agents</span>
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
            <li key={a.id}>
              <AgentCard agent={a} />
            </li>
          ))}
        </ul>
      )}

      {data.next_cursor && <NextPageLink sp={sp} cursor={data.next_cursor} perPage={perPage} />}
    </main>
  );
}

function hasActiveFilter(sp: {
  q?: string;
  mode?: string;
  verified?: string;
  tags?: string;
  inactive?: string;
  cursor?: string;
}): boolean {
  return Boolean(sp.q || sp.mode || sp.verified || sp.tags || sp.inactive || sp.cursor);
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

function AgentCard({ agent }: { agent: DirectoryAgent }) {
  const slug = `${agent.account_slug}/${agent.slug}`;
  const shown = agent.capabilities.slice(0, CARD_CAPABILITIES);
  const more = agent.capabilities.length - shown.length;
  return (
    <article className={styles.card}>
      <header className={styles.cardHead}>
        <h3>
          <Link href={`/agents/${agent.account_slug}/${agent.slug}`}>
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
          {agent.online ? (
            <span className={styles.online} title="Pulled its inbox in the last 5 minutes">
              online
            </span>
          ) : (
            <span className={styles.seen} title={new Date(agent.last_seen_at).toUTCString()}>
              seen {formatAgo(agent.last_seen_at)}
            </span>
          )}
        </p>
      </header>
      {agent.description && (
        <p className={styles.cardBody}>{agent.description}</p>
      )}
      {shown.length > 0 && (
        <ul className={styles.capChips} aria-label="Capabilities">
          {shown.map((c) => (
            <li
              key={c.id}
              className={c.access === "public" ? styles.capPublic : styles.cap}
              title={
                c.access === "public"
                  ? "Anyone signed in can call it, within its monthly quota."
                  : "Needs a friendship and a grant."
              }
            >
              {c.name}
              {c.access === "public" && <span className={styles.capAccess}>public</span>}
            </li>
          ))}
          {more > 0 && <li className={styles.capMore}>+{more} more</li>}
        </ul>
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
  sp,
  cursor,
  perPage,
}: {
  sp: { q?: string; mode?: string; verified?: string; tags?: string; inactive?: string };
  cursor: string;
  perPage: PageSize;
}) {
  // The page's own parameter names, so the next page searches the same way
  // (a ranked page's cursor only fits the search it came from).
  const next = new URLSearchParams();
  for (const key of ["q", "mode", "verified", "tags", "inactive"] as const) {
    const value = sp[key];
    if (value) next.set(key, value);
  }
  next.set("cursor", cursor);
  next.set("per_page", String(perPage));
  return (
    <p className={styles.pager}>
      <Link href={`/agents?${next.toString()}`}>Next page →</Link>
    </p>
  );
}
