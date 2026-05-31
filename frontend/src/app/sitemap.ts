import type { MetadataRoute } from "next";

const BASE = "https://chakramcp.com";

/**
 * Generates /sitemap.xml.
 *
 * Lists the static, indexable marketing + docs routes. Deliberately
 * excluded:
 *
 *   - `/brand`, `/cofounder`, `/concept` — unlisted pages carrying
 *     per-page `robots: { index: false }`. Listing them here would
 *     defeat the "unlisted" intent (same reasoning as robots.ts).
 *   - `/agents/[account_slug]/[agent_slug]` — per-agent pages are
 *     user-generated and unbounded; enumerating them needs a DB query
 *     at build/request time. They're reachable via links from the
 *     `/agents` directory, so crawlers still discover them. If we ever
 *     want them in the sitemap, generate a second dynamic sitemap via
 *     `generateSitemaps()` backed by a network-agents query.
 *
 * `priority` is relative-within-site (Google treats it as a hint, not
 * a ranking signal): home + directory highest, docs next, terms low.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const now = new Date();
  return [
    { url: `${BASE}/`, lastModified: now, changeFrequency: "weekly", priority: 1.0 },
    { url: `${BASE}/agents`, lastModified: now, changeFrequency: "daily", priority: 0.9 },
    { url: `${BASE}/docs`, lastModified: now, changeFrequency: "weekly", priority: 0.8 },
    { url: `${BASE}/docs/quickstart`, lastModified: now, changeFrequency: "weekly", priority: 0.8 },
    { url: `${BASE}/docs/concepts`, lastModified: now, changeFrequency: "monthly", priority: 0.7 },
    { url: `${BASE}/docs/agents`, lastModified: now, changeFrequency: "monthly", priority: 0.7 },
    { url: `${BASE}/terms`, lastModified: now, changeFrequency: "yearly", priority: 0.3 },
  ];
}
