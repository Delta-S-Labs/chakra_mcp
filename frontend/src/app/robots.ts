import type { MetadataRoute } from "next";

/**
 * Generates /robots.txt.
 *
 * We deliberately do NOT list the unlisted pages (/concept, /brand,
 * /cofounder, /render/*) here, because anyone who reads /robots.txt
 * would learn that they exist. That defeats the point of "unlisted."
 *
 * Those pages rely on per-page `robots: { index: false, follow: false }`
 * metadata (rendered as <meta name="robots" content="noindex, nofollow">)
 * to keep them out of search results. Compliant crawlers honor that.
 */
export default function robots(): MetadataRoute.Robots {
  return {
    rules: [{ userAgent: "*", allow: "/" }],
  };
}
