import type { NextConfig } from "next";

const securityHeaders = [
  { key: "X-Frame-Options", value: "DENY" },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
];

const nextConfig: NextConfig = {
  // Seobility flagged the `X-Powered-By: Next.js` response header —
  // it leaks the framework and adds nothing. Next only honours this
  // flag for routes it serves itself, which is all of them here.
  poweredByHeader: false,
  // PostHog ingestion is reverse-proxied through our own origin (see
  // the `/ingest` rewrites below): events stay first-party and survive
  // ad-blockers. `skipTrailingSlashRedirect` keeps Next from 308-ing
  // PostHog's trailing-slash API paths (e.g. /decide) before the
  // rewrite runs.
  skipTrailingSlashRedirect: true,
  async rewrites() {
    return [
      // Sitemap alias: serve the generated /sitemap.xml at the
      // underscored /site_map.xml too. A rewrite (not a redirect) so
      // the alias returns the XML directly with a 200 — every crawler
      // and Search-Console-style submission accepts it without having
      // to follow a redirect. Canonical URL stays /sitemap.xml (that's
      // what robots.txt advertises); this is just a second working URL.
      {
        source: "/site_map.xml",
        destination: "/sitemap.xml",
      },
      // Static assets (array bundles, recorder, surveys, toolbar).
      {
        source: "/ingest/static/:path*",
        destination: "https://us-assets.i.posthog.com/static/:path*",
      },
      // Event capture, /decide, /flags, etc. Keep this AFTER the static
      // rule so /ingest/static/* doesn't fall through to the API host.
      {
        source: "/ingest/:path*",
        destination: "https://us.i.posthog.com/:path*",
      },
    ];
  },
  async headers() {
    return [
      {
        source: "/:path*",
        headers: securityHeaders,
      },
    ];
  },
  async redirects() {
    return [
      // Legacy public onboard surface → authed pair surface.
      // Per Next.js docs: "When a redirect is applied, any query values
      // provided in the request will be passed through to the redirect
      // destination" — so `?session=ABCD-1234` rides along automatically.
      {
        source: "/onboard",
        destination: "/app/pair",
        permanent: true,
      },
      // Discovery breadcrumb: an earlier revision of
      // /.well-known/chakramcp.json (and Hermes' tooling) pointed at
      // these URLs on the marketing host even though the backend lives
      // on `app.chakramcp.com`. Both 404'd. The descriptor is now
      // correct, but redirect here for anyone with the old URL
      // hardcoded in their config (which Hermes hit on cli-v0.1.0).
      // 308 preserves the GET — clients can transparently follow.
      {
        source: "/.well-known/oauth-authorization-server",
        destination:
          "https://app.chakramcp.com/.well-known/oauth-authorization-server",
        permanent: true,
        basePath: false,
      },
      {
        source: "/.well-known/oauth-protected-resource",
        destination:
          "https://relay.chakramcp.com/.well-known/oauth-protected-resource",
        permanent: true,
        basePath: false,
      },
    ];
  },
};

export default nextConfig;
