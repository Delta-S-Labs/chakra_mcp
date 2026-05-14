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
    ];
  },
};

export default nextConfig;
