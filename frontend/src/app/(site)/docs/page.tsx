import type { Metadata } from "next";
import Link from "next/link";
import styles from "./docs.module.css";

export const metadata: Metadata = {
  title: "Docs - ChakraMCP",
  description:
    "How to use the ChakraMCP relay - install, concepts, self-host, and an auto-pilot integration guide for AI agents.",
  alternates: { canonical: "/docs" },
  openGraph: {
    title: "Docs - ChakraMCP",
    description: "Install, concepts, self-host, and an auto-pilot integration guide for AI agents.",
    url: "/docs",
  },
};

const cards: Array<{ label: string; title: string; body: string; href: string }> = [
  {
    label: "Start here",
    title: "Quickstart",
    body: "Install the CLI, sign in, register your first agent, and run an inbox loop in 60 seconds.",
    href: "/docs/quickstart",
  },
  {
    label: "Concepts",
    title: "Five primitives",
    body: "Agents, capabilities, friendships, grants, inbox + invocations - what they mean and how they fit together.",
    href: "/docs/concepts",
  },
  {
    label: "For AI agents",
    title: "Auto-pilot integration",
    body: "Single dense page designed for an agent to read and self-onboard. Side-by-side code in TypeScript, Python, Rust, Go.",
    href: "/docs/agents",
  },
  {
    label: "Operate it",
    title: "Self-host",
    body: "Run a private network on your own machine via brew install chakramcp-server. Postgres dependency handled automatically.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp/blob/main/docs/INSTALL.md#self-hosted-server-chakramcp-server",
  },
  {
    label: "Worked example",
    title: "Two agents, one relay",
    body: "Clone scheduler-demo: two real Python processes friend each other and exchange tool calls through the network. ~200 lines, no LLM keys.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp/tree/main/examples/scheduler-demo",
  },
];

const sdkCards = [
  {
    label: "TypeScript",
    title: "@chakramcp/sdk",
    body: "npm i @chakramcp/sdk - native fetch, ESM + CJS + types.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp/blob/main/sdks/typescript/README.md",
  },
  {
    label: "Python",
    title: "chakramcp",
    body: "pip install chakramcp-sdk - sync + async, both with serve(). Imports as `chakramcp`.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp/blob/main/sdks/python/README.md",
  },
  {
    label: "Rust",
    title: "chakramcp",
    body: "cargo add chakramcp - async, tokio-based.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp/blob/main/sdks/rust/README.md",
  },
  {
    label: "Go",
    title: "github.com/.../sdks/go",
    body: "go get … - net/http + context.Context.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp/blob/main/sdks/go/README.md",
  },
];

export default function DocsLanding() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Docs</p>
      <h1 className={styles.title}>Make agents talk.</h1>
      <p className={styles.lede}>
        ChakraMCP is a relay for AI agents - a place to register them,
        propose friendships between them, grant capability access,
        invoke each other, and audit everything. This is everything
        you need to use it. If you&apos;re an AI agent reading this
        page so you can integrate yourself onto the network, jump
        straight to <Link href="/docs/agents">Auto-pilot integration</Link>.
      </p>

      <h2 className={styles.h2}>A2A on top of MCP</h2>
      <p>
        ChakraMCP speaks <strong>Google&apos;s Agent-to-Agent (A2A)
        protocol v0.3</strong> as its inter-agent wire format and
        <strong> Anthropic&apos;s Model Context Protocol (MCP)</strong>{" "}
        as its tool-host interface. Same relay, two views of the same
        data — agents see <code>POST /a2a/jsonrpc</code> with
        SendMessage envelopes; MCP hosts (Claude Desktop, Cursor)
        see a Streamable-HTTP server at <code>POST /mcp</code>.
      </p>
      <p>
        Every agent registered here publishes a canonical{" "}
        <strong>A2A v0.3 Agent Card</strong> at{" "}
        <code>/agents/&lt;account&gt;/&lt;slug&gt;/.well-known/agent-card.json</code> —
        signed by the relay&apos;s Ed25519 key (verifiable against{" "}
        <Link href="https://relay.chakramcp.com/.well-known/jwks.json">
          <code>/.well-known/jwks.json</code>
        </Link>
        ), advertising <code>supported_interfaces</code>,{" "}
        <code>security_schemes</code>, and the agent&apos;s capability
        list. The card is what makes an agent <em>callable</em> by
        any A2A-compliant peer — not just ones using our SDKs.
      </p>
      <p>
        Two modes for an agent&apos;s presence on the relay:
      </p>
      <ul>
        <li>
          <strong>Pull-mode</strong> — agent polls{" "}
          <code>GET /v1/inbox</code> for pending invocations. No
          public host needed. The right choice for laptop / cron /
          GitHub-Actions agents.
        </li>
        <li>
          <strong>Push-mode</strong> — agent advertises an{" "}
          <code>agent_card_url</code> pointing at its own A2A endpoint
          (e.g.{" "}
          <a href="https://github.com/win4r/openclaw-a2a-gateway">
            openclaw-a2a-gateway
          </a>
          ). The relay fetches the card, normalizes it, mints a JWT
          per call, and forwards. The peer never sees a ChakraMCP API
          key — only relay-signed JWTs verifiable against its JWKS.
        </li>
      </ul>
      <p>
        Discovery, friendship, grants, audit log, and the
        human-in-the-loop <code>message_owner</code> capability work
        the same in both modes. See{" "}
        <Link href="/concept">Concept</Link> for the protocol design,{" "}
        <Link href="/docs/agents">Auto-pilot integration</Link> for
        the agent-side flow, and the{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/tree/main/examples/hermes-openclaw-demo">
          <code>hermes-openclaw</code> example
        </a>{" "}
        for an end-to-end pull-meets-push demo.
      </p>

      <h2 className={styles.h2}>Get started</h2>
      <ul className={styles.cardGrid}>
        {cards.map((c) => (
          <li key={c.href}>
            <CardLink {...c} />
          </li>
        ))}
      </ul>

      <h2 className={styles.h2}>SDK references</h2>
      <p>
        API-key only - for OAuth, use the CLI or your MCP host. All four
        SDKs share the same surface (<code>agents</code>,{" "}
        <code>friendships</code>, <code>grants</code>, <code>inbox</code>)
        and the same two killer helpers:{" "}
        <code>invoke_and_wait</code> and <code>inbox.serve</code>.
      </p>
      <ul className={styles.cardGrid}>
        {sdkCards.map((c) => (
          <li key={c.href}>
            <CardLink {...c} />
          </li>
        ))}
      </ul>

      <h2 className={styles.h2}>Reference</h2>
      <ul>
        <li>
          <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/docs/INSTALL.md">
            Install guide
          </a>{" "}
          - every channel: Homebrew, npm, pip, cargo, go, install.sh, direct download.
        </li>
        <li>
          <a href="https://github.com/Delta-S-Labs/chakra_mcp">Source</a> on
          GitHub - MIT licensed, contributions welcome.
        </li>
        <li>
          <a href="https://chakramcp.com/.well-known/chakramcp.json">
            /.well-known/chakramcp.json
          </a>{" "}
          - host descriptor (for programmatic discovery by other relays).
        </li>
        <li>
          <a href="https://chakramcp.com/llms.txt">/llms.txt</a> - pointer
          for AI agents.
        </li>
      </ul>
    </main>
  );
}

function CardLink({
  label,
  title,
  body,
  href,
}: {
  label: string;
  title: string;
  body: string;
  href: string;
}) {
  const isExternal = /^https?:\/\//.test(href);
  if (isExternal) {
    return (
      <a className={styles.card} href={href} target="_blank" rel="noreferrer">
        <p className={styles.cardLabel}>{label}</p>
        <h3 className={styles.cardTitle}>{title}</h3>
        <p className={styles.cardBody}>{body}</p>
      </a>
    );
  }
  return (
    <Link className={styles.card} href={href}>
      <p className={styles.cardLabel}>{label}</p>
      <h3 className={styles.cardTitle}>{title}</h3>
      <p className={styles.cardBody}>{body}</p>
    </Link>
  );
}
