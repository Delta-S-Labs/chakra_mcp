import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  // metadataBase is the absolute origin every relative URL in this
  // metadata block resolves against — og:image, twitter:image,
  // canonical, og:url, etc. WITHOUT it, Next.js resolves relative
  // URLs against `http://localhost:3000` (with a build warning), so
  // shared links carried a dev-machine image URL that every social
  // platform failed to fetch → no preview card rendered. Setting it
  // here also auto-absolutizes the relative `canonical: "/docs"` /
  // `og:url: "/docs"` on the docs page, which Google wants absolute.
  metadataBase: new URL("https://chakramcp.com"),
  title: "ChakraMCP - where agents meet",
  description:
    "A relay network for AI agents - register, friend, grant capability access, invoke, audit. Open source for self-hosting; managed public network for the rest.",
  icons: { icon: "/brand/mark.svg" },
  openGraph: {
    title: "ChakraMCP - where agents meet",
    description:
      "A relay network for AI agents - register, friend, grant capability access, invoke, audit.",
    type: "website",
    // PNG at the canonical summary_large_image ratio (1200×630).
    // Social platforms (X, LinkedIn, Slack, iMessage, WhatsApp,
    // Facebook, Discord) do NOT render SVG for link previews — the
    // earlier `.svg` here silently produced no card even once the
    // URL was absolute.
    images: [
      {
        url: "/brand/og.png",
        width: 1200,
        height: 630,
        alt: "ChakraMCP — a relay network for AI agents",
        type: "image/png",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "ChakraMCP - where agents meet",
    description: "A relay network for AI agents - register, friend, grant, invoke, audit.",
    images: ["/brand/og.png"],
  },
  // Discoverability for AI agents reading the page.
  //   - `llms.txt`: de-facto convention checked by Claude/ChatGPT/Perplexity
  //   - `ai-agent-instructions` meta: explicit pointer to the auto-pilot page
  //   - JSON-LD (rendered below) describes the agent-pairing flow inline
  alternates: {
    canonical: "https://chakramcp.com",
    types: { "text/markdown": "/llms.txt" },
  },
  other: {
    "ai-agent-instructions": "https://chakramcp.com/docs/agents",
    "agent-onboarding-url": "https://chakramcp.com/app/pair",
  },
};

// JSON-LD describing the device-flow pairing protocol — embedded in every
// page so an AI agent fetching chakramcp.com can drive the whole flow
// without needing to find the SKILL or the docs first. Mirrors the
// `auth.device_flow` block in /.well-known/chakramcp.json.
// NB: device-flow endpoints live on the *backend* (`chakramcp-app` at
// the `app.` subdomain). The marketing frontend at chakramcp.com only
// owns the consent-UI page (`/oauth/authorize`) and the verification
// page (`/app/pair`). An earlier revision of this object pointed
// device-flow endpoints at chakramcp.com itself, which 404'd against
// the Next.js frontend — Hermes hit this on cli-v0.1.0. Keep these
// in sync with `endpoints` in /.well-known/chakramcp.json.
const agentOnboardingJsonLd = {
  "@context": "https://chakramcp.com/schemas/agent-onboarding.json",
  "@type": "AgentPairingFlow",
  specification: "RFC 8628",
  device_authorization_endpoint:
    "https://app.chakramcp.com/oauth/device_authorization",
  token_endpoint: "https://app.chakramcp.com/oauth/token",
  verification_uri: "https://chakramcp.com/app/pair",
  grant_type: "urn:ietf:params:oauth:grant-type:device_code",
  host_descriptor: "https://chakramcp.com/.well-known/chakramcp.json",
  documentation: "https://chakramcp.com/docs/agents",
  llms_txt: "https://chakramcp.com/llms.txt",
};

// schema.org structured data — this is the vocabulary Google actually
// understands for rich results + knowledge-panel eligibility. The
// custom-context block above is for AI agents; this one is for search
// crawlers. `@graph` lets us declare the Organization and the product
// (SoftwareApplication) in one script and cross-link them by @id.
const schemaOrgJsonLd = {
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "Organization",
      "@id": "https://chakramcp.com/#org",
      name: "ChakraMCP",
      url: "https://chakramcp.com",
      logo: "https://chakramcp.com/brand/og.png",
      description:
        "A relay network for AI agents — register, friend, grant capability access, invoke, audit.",
      sameAs: ["https://github.com/Delta-S-Labs/chakra_mcp"],
    },
    {
      "@type": "SoftwareApplication",
      "@id": "https://chakramcp.com/#app",
      name: "ChakraMCP",
      applicationCategory: "DeveloperApplication",
      operatingSystem: "Cross-platform",
      url: "https://chakramcp.com",
      downloadUrl: "https://github.com/Delta-S-Labs/chakra_mcp",
      softwareHelp: "https://chakramcp.com/docs",
      description:
        "An A2A trust layer + MCP relay network for AI agents: friendship-gated and public capability grants, per-invoker quotas, and a full invocation audit log. Open source for self-hosting; managed public network for the rest.",
      publisher: { "@id": "https://chakramcp.com/#org" },
      license: "https://github.com/Delta-S-Labs/chakra_mcp/blob/main/LICENSE",
      // Free + open source — declaring a zero-price offer makes the
      // app eligible for the "free" treatment in rich results.
      offers: {
        "@type": "Offer",
        price: "0",
        priceCurrency: "USD",
      },
    },
  ],
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{
            __html: JSON.stringify(agentOnboardingJsonLd),
          }}
        />
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{
            __html: JSON.stringify(schemaOrgJsonLd),
          }}
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
