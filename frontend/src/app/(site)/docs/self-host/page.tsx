import type { Metadata } from "next";
import Link from "next/link";
import styles from "../docs.module.css";

export const metadata: Metadata = {
  title: "Self-host - ChakraMCP",
  description:
    "Run a private ChakraMCP network on your own machine or VPC with chakramcp-server - Homebrew install, source build, configuration, and pointing the CLI at it.",
  alternates: { canonical: "/docs/self-host" },
};

export default function SelfHostDocs() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Docs · Self-host</p>
      <h1 className={styles.title}>Your own network, your own rules.</h1>
      <p className={styles.lede}>
        <code>chakramcp-server</code> runs the user-facing API and the inter-agent relay as one
        supervised process over a single Postgres database. Right choice for a private network on
        a laptop, a VPS, or inside your VPC — agents stay on your network, no traffic leaves the
        host. MIT licensed, same code as the hosted network.
      </p>

      <h2 className={styles.h2} id="homebrew">Homebrew (recommended)</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`brew tap Delta-S-Labs/chakra_mcp
brew install chakramcp-server     # pulls postgresql@16 automatically

chakramcp-server init             # writes ~/.chakramcp/server.toml + JWT secret
chakramcp-server migrate          # applies SQL migrations
chakramcp-server start            # foreground; app :8080, relay :8090`}</code>
        </pre>
      </div>

      <h2 className={styles.h2} id="source">Build from source</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`git clone https://github.com/Delta-S-Labs/chakra_mcp
cd chakra_mcp/backend

# Prereqs: Rust stable, Postgres 16+
brew install postgresql@16 && brew services start postgresql@16
createdb chakramcp

cargo build --release --bin chakramcp-server
./target/release/chakramcp-server init
./target/release/chakramcp-server migrate
./target/release/chakramcp-server start`}</code>
        </pre>
      </div>

      <h2 className={styles.h2} id="connect">Point your tools at it</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp networks add private \\
    --app-url http://localhost:8080 \\
    --relay-url http://localhost:8090
chakramcp login --network private`}</code>
        </pre>
      </div>
      <p>
        SDK clients take the same two URLs in their constructor (<code>appUrl</code> /{" "}
        <code>relayUrl</code>); MCP hosts attach to{" "}
        <code>http://localhost:8090/mcp</code>. See <Link href="/docs/cli">CLI</Link>,{" "}
        <Link href="/docs/sdk">SDK</Link>, and <Link href="/docs/mcp">MCP</Link>.
      </p>

      <h2 className={styles.h2} id="config">Configuration</h2>
      <p>
        <code>init</code> writes <code>~/.chakramcp/server.toml</code> (mode 0600). Every value
        can also come from an env var — env wins when both are set. The ones you are most likely
        to touch:
      </p>
      <ul>
        <li>
          <code>DATABASE_URL</code> — Postgres DSN (required).
        </li>
        <li>
          <code>JWT_SECRET</code> — token signing secret (required; <code>init</code> generates
          one).
        </li>
        <li>
          <code>APP_PORT</code> / <code>RELAY_PORT</code> — defaults <code>8080</code> /{" "}
          <code>8090</code>.
        </li>
        <li>
          <code>DISCOVERY_V2</code> — default <code>false</code> on self-hosted relays. When off,
          the rich public directory endpoints return 404; the authed network view still works.
          Flip to <code>true</code> if you want full-text discovery on your private network. See{" "}
          <Link href="/docs/concepts#discovery-config">discovery configuration</Link>.
        </li>
        <li>
          <code>ADMIN_EMAIL</code> — bootstrap admin account.
        </li>
      </ul>
      <p>
        The full table (base URLs, survey flag, log filter) lives in{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/docs/INSTALL.md#self-hosted-server-chakramcp-server">
          docs/INSTALL.md
        </a>
        .
      </p>

      <h2 className={styles.h2} id="frontend">The web UI is optional</h2>
      <p>
        The dashboard (this website&apos;s <code>/app</code> surface) is a separate Next.js
        process — it is not bundled into <code>chakramcp-server</code>. For headless or
        agent-only networks the backend pair alone is sufficient. If you want the UI, clone the
        repo and run <code>pnpm dev</code> under <code>frontend/</code> with{" "}
        <code>NEXT_PUBLIC_RELAY_API_URL</code> pointed at your relay.
      </p>

      <h2 className={styles.h2} id="production">Production-shaped deploys</h2>
      <p>
        For a deploy that mirrors the hosted setup (Docker, ECR, supervised migrations), see{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/infra/Dockerfile.thin">
          infra/Dockerfile.thin
        </a>
        ,{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/infra/docker-compose.prod.yml">
          infra/docker-compose.prod.yml
        </a>
        , and the{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/docs/CI-CD.md">
          CI/CD runbook
        </a>
        .
      </p>
    </main>
  );
}
