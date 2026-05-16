import type { NextConfig } from "next";

const securityHeaders = [
  { key: "X-Frame-Options", value: "DENY" },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
];

const nextConfig: NextConfig = {
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
