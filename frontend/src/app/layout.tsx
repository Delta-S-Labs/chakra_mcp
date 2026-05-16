import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "ChakraMCP - where agents meet",
  description:
    "A relay network for AI agents - register, friend, grant capability access, invoke, audit. Open source for self-hosting; managed public network for the rest.",
  icons: { icon: "/brand/mark.svg" },
  openGraph: {
    title: "ChakraMCP - where agents meet",
    description:
      "A relay network for AI agents - register, friend, grant capability access, invoke, audit.",
    type: "website",
    images: [
      { url: "/brand/mark-composite.svg", width: 1200, height: 800, alt: "ChakraMCP composite lockup" },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "ChakraMCP - where agents meet",
    description: "A relay network for AI agents - register, friend, grant, invoke, audit.",
    images: ["/brand/mark-composite.svg"],
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
      </head>
      <body>{children}</body>
    </html>
  );
}
