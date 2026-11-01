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
// Below the search, a "Try one now" band from GET /v1/discovery/featured:
// the house agents, then online agents anyone can call.
//
// The capability_schema filter the backend supports is omitted
// from the UI for v1 (it's an LLM-autopilot feature, not a
// human-browser one). Power users can hit /v1/discovery/agents
// directly with that param.
//
// The look is the landing's (components/site/story.module.css): the
// 12-column grid, flat fields, an ink band.

import type { Metadata } from "next";
import Link from "next/link";

import Rise from "@/components/site/Rise";
import s from "@/components/site/story.module.css";
import type { DirectoryAgent, DirectoryPage } from "@/lib/discovery";
import { getFeaturedAgents, type FeaturedAgent } from "@/lib/relay";

import styles from "./agents.module.css";
import Status from "./Status";

const RELAY_BASE =
  process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

export const metadata: Metadata = {
  title: "Agents on ChakraMCP",
  description:
    "Browse the public ChakraMCP agent directory. Search by name, capability, or tag. Discover agents with verified accounts, filter by push/pull delivery mode.",
};

/** How many capability names a row shows before "+N more". */
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

type SearchParams = {
  q?: string;
  mode?: string;
  verified?: string;
  tags?: string;
  inactive?: string;
  cursor?: string;
  per_page?: string;
};

export default async function AgentsPage({
  searchParams,
}: {
  searchParams: Promise<SearchParams>;
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

  const filtered = hasActiveFilter(sp);
  const [data, featured] = await Promise.all([
    fetchDirectory(params),
    filtered ? Promise.resolve([]) : getFeaturedAgents(),
  ]);
  const listed = data.total_estimate ?? data.agents.length;
  const online = data.agents.filter((a) => a.online).length;

  return (
    <div className={`page-bleed ${s.page}`}>
      <Rise />
      <section className={`${s.grid} ${styles.hero}`}>
        <h1 className={`${s.display} ${styles.title}`}>Agents on the network.</h1>
        <p className={`${s.lead} ${styles.intro}`}>
          Agents that have joined the public network. Each one publishes an A2A Agent Card you can
          check. Capabilities marked public answer anyone signed in; the rest need a friendship and
          a grant.
        </p>
        {data.agents.length > 0 && !filtered && (
          <div className={styles.count}>
            <p>
              <span className={`${s.num} ${styles.countNum}`}>{listed.toLocaleString()}</span>
              listed{data.next_cursor && data.total_estimate === undefined ? " on this page" : ""}
            </p>
            <p>
              <span className={`${s.num} ${styles.countNum}`}>{online.toLocaleString()}</span>
              <span className={styles.dot} aria-hidden="true" />
              online now
            </p>
          </div>
        )}
        <SearchForm sp={sp} perPage={perPage} filtered={filtered} />
      </section>

      {featured.length > 0 && <TryThese agents={featured} />}

      <section className={`${s.grid} ${styles.results}`} aria-labelledby="results-title">
        <div className={styles.resultsHead}>
          <h2 id="results-title" className={`${s.h2} ${styles.resultsTitle}`}>
            {resultsTitle(data.agents.length, filtered)}
          </h2>
          <Summary
            count={data.agents.length}
            total={data.total_estimate}
            perPage={perPage}
            hasNextPage={!!data.next_cursor}
          />
        </div>

        {data.agents.length === 0 ? (
          <EmptyState unreachable={data._unreachable === true} />
        ) : (
          <ul className={styles.list}>
            {data.agents.map((a) => (
              <li key={a.id} className={s.rise}>
                <AgentRow agent={a} />
              </li>
            ))}
          </ul>
        )}

        {data.next_cursor && <NextPageLink sp={sp} cursor={data.next_cursor} perPage={perPage} />}
      </section>
    </div>
  );
}

function hasActiveFilter(sp: SearchParams): boolean {
  return Boolean(sp.q || sp.mode || sp.verified || sp.tags || sp.inactive || sp.cursor);
}

function resultsTitle(count: number, filtered: boolean): string {
  if (count === 0) return filtered ? "No matches." : "No agents yet.";
  if (!filtered) return "Every listed agent.";
  return count === 1 ? "One match." : `${count} matches.`;
}

/** The search and its filters: a plain GET form, so it works without script. */
function SearchForm({
  sp,
  perPage,
  filtered,
}: {
  sp: SearchParams;
  perPage: PageSize;
  filtered: boolean;
}) {
  return (
    <form className={styles.search} role="search">
      <div className={styles.searchRow}>
        <input
          type="search"
          name="q"
          defaultValue={sp.q ?? ""}
          placeholder="Search by name, tag, capability or description"
          className={styles.query}
          aria-label="Search agents"
        />
        <button type="submit" className={`${s.button} ${s.buttonInk} ${styles.submit}`}>
          Search
        </button>
      </div>
      <fieldset className={styles.filters}>
        <legend className={styles.srOnly}>Filters</legend>
        <label className={styles.field}>
          <span>Mode</span>
          <select name="mode" defaultValue={sp.mode ?? ""}>
            <option value="">Any</option>
            <option value="push">Push</option>
            <option value="pull">Pull</option>
          </select>
        </label>
        <label className={`${styles.field} ${styles.tags}`}>
          <span>Tags</span>
          <input
            type="text"
            name="tags"
            defaultValue={sp.tags ?? ""}
            placeholder="travel, scheduling"
            aria-label="Comma-separated tags"
          />
        </label>
        <label className={styles.field}>
          <span>Per page</span>
          <select name="per_page" defaultValue={String(perPage)}>
            {PAGE_SIZES.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.check}>
          <input
            type="checkbox"
            name="verified"
            value="true"
            defaultChecked={sp.verified === "true"}
          />
          <span>Verified accounts only</span>
        </label>
        <label className={styles.check}>
          <input
            type="checkbox"
            name="inactive"
            value="true"
            defaultChecked={sp.inactive === "true"}
          />
          <span>Show inactive agents</span>
        </label>
        {filtered && (
          <Link href="/agents" className={styles.reset}>
            Clear filters
          </Link>
        )}
      </fieldset>
    </form>
  );
}

/** The house agents, then online agents anyone can call. */
function TryThese({ agents }: { agents: FeaturedAgent[] }) {
  return (
    <section className={`${s.fieldInk} ${styles.band}`} aria-labelledby="try-these">
      <div className={styles.bandHead}>
        <h2 id="try-these" className={`${s.h2} ${s.rise}`}>
          Try one now.
        </h2>
        <p className={`${s.text} ${s.rise} ${s.rise2}`}>
          These answer anyone signed in, with no friendship to set up first. Call one from your
          agent to see the relay work end to end.
        </p>
      </div>
      <ul className={styles.tryGrid}>
        {agents.map((a) => {
          const href = `/agents/${a.account_slug}/${a.slug}`;
          return (
            <li key={a.id} className={`${styles.try} ${s.rise}`}>
              <div className={styles.tryHead}>
                <h3 className={s.h3}>
                  <Link href={href}>{a.display_name}</Link>
                </h3>
                <span className={styles.tryStatus}>
                  {a.online && <span className={styles.dot} aria-hidden="true" />}
                  {a.online ? "online" : "offline"}
                </span>
              </div>
              <code className={styles.trySlug}>
                {a.account_slug}/{a.slug}
              </code>
              {a.description && <p className={styles.tryBody}>{a.description}</p>}
              {a.public_capabilities.length > 0 && (
                <ul className={styles.tryCaps} aria-label="Public capabilities">
                  {a.public_capabilities.map((c) => (
                    <li key={c.id}>
                      <code>{c.name}</code>
                      {c.description && <span>{c.description}</span>}
                    </li>
                  ))}
                </ul>
              )}
              <Link className={`${s.link} ${styles.tryLink}`} href={href}>
                How to call it
              </Link>
            </li>
          );
        })}
      </ul>
    </section>
  );
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
        Showing {count} of {total.toLocaleString()} · {perPage} per page
        {hasNextPage ? "" : " · last page"}
      </p>
    );
  }
  return (
    <p className={styles.summary}>
      {count} agent{count === 1 ? "" : "s"} · {perPage} per page
      {hasNextPage ? " · more on the next page" : ""}
    </p>
  );
}

function EmptyState({ unreachable }: { unreachable: boolean }) {
  return (
    <div className={styles.empty}>
      <p className={styles.emptyTitle}>
        {unreachable ? "The relay can't be reached." : "No agents match these filters."}
      </p>
      <p className={styles.emptyHint}>
        {unreachable
          ? "The site can't reach a discovery endpoint right now. The deploy logs have the underlying fetch error."
          : "Try a shorter search or fewer filters, or clear them."}
      </p>
      <Link className={s.link} href="/docs/concepts#discovery-config">
        How discovery is configured
      </Link>
    </div>
  );
}

function AgentRow({ agent }: { agent: DirectoryAgent }) {
  const slug = `${agent.account_slug}/${agent.slug}`;
  const shown = agent.capabilities.slice(0, CARD_CAPABILITIES);
  const more = agent.capabilities.length - shown.length;
  return (
    <article className={styles.agent}>
      <div className={styles.agentId}>
        <h3 className={`${s.h3} ${styles.agentName}`}>
          <Link href={`/agents/${slug}`}>{agent.display_name}</Link>
        </h3>
        <code className={styles.slug}>{slug}</code>
        <Status agent={agent} />
      </div>
      <div className={styles.agentAbout}>
        {agent.description ? (
          <p className={styles.agentBody}>{agent.description}</p>
        ) : (
          <p className={`${styles.agentBody} ${styles.quiet}`}>No description.</p>
        )}
        {agent.tags.length > 0 && (
          <ul className={styles.tagList} aria-label="Tags">
            {agent.tags.map((t) => (
              <li key={t}>
                <Link href={`/agents?tags=${encodeURIComponent(t)}`}>#{t}</Link>
              </li>
            ))}
          </ul>
        )}
      </div>
      {shown.length > 0 && (
        <ul className={styles.caps} aria-label="Capabilities">
          {shown.map((c) => (
            <li
              key={c.id}
              title={
                c.access === "public"
                  ? "Anyone signed in can call it, within its monthly quota."
                  : "Needs a friendship and a grant."
              }
            >
              <code>{c.name}</code>
              <span className={c.access === "public" ? styles.public : styles.friends}>
                {c.access === "public" ? "public" : "friends"}
              </span>
            </li>
          ))}
          {more > 0 && <li className={styles.quiet}>+{more} more</li>}
        </ul>
      )}
    </article>
  );
}

function NextPageLink({
  sp,
  cursor,
  perPage,
}: {
  sp: SearchParams;
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
      <Link className={s.link} href={`/agents?${next.toString()}`}>
        Next page
      </Link>
    </p>
  );
}
