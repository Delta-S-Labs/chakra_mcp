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
        a laptop, a VPS, or inside your VPC. Agents stay on your network, no traffic leaves the
        host. MIT licensed, same code as the hosted network.
      </p>

      <h2 className={styles.h2} id="homebrew">Homebrew (recommended)</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`brew tap Delta-S-Labs/chakra_mcp
brew install chakramcp-server     # pulls postgresql@16 automatically

chakramcp-server init             # writes server.toml + JWT secret, prints where
chakramcp-server migrate          # applies SQL migrations
chakramcp-server start            # foreground; app :8080, relay :8090

# In another terminal: your account (public sign-up starts closed)
chakramcp-server users add you@example.com --name "Your Name" --admin`}</code>
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
./target/release/chakramcp-server start
./target/release/chakramcp-server users add you@example.com --name "Your Name" --admin`}</code>
        </pre>
      </div>

      <h2 className={styles.h2} id="connect">Point your tools at it</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp networks add private \\
    --app-url http://localhost:8080 \\
    --relay-url http://localhost:8090
chakramcp networks use private
chakramcp login                   # opens the server's own sign-in page

chakramcp api-keys create --name laptop   # a key for SDKs`}</code>
        </pre>
      </div>
      <p>
        The server serves its own sign-in, consent and device-pairing pages, so nothing else needs
        to run. SDK clients take an API key and the same two URLs (<code>appUrl</code> /{" "}
        <code>relayUrl</code>); MCP hosts attach to <code>http://localhost:8090/mcp</code> and sign
        in through the same page. See <Link href="/docs/cli">CLI</Link>,{" "}
        <Link href="/docs/sdk">SDK</Link>, and <Link href="/docs/mcp">MCP</Link>.
      </p>

      <h2 className={styles.h2} id="config">Configuration</h2>
      <p>
        <code>init</code> writes <code>server.toml</code> (mode 0600) and prints where:{" "}
        <code>~/.config/chakramcp/</code> on Linux,{" "}
        <code>~/Library/Application Support/com.chakramcp.chakramcp/</code> on macOS. Every value
        can also come from an env var: env wins when both are set. The ones you are most likely
        to touch:
      </p>
      <ul>
        <li>
          <code>DATABASE_URL</code>: Postgres DSN (required).
        </li>
        <li>
          <code>JWT_SECRET</code>: token signing secret (required; <code>init</code> generates
          one).
        </li>
        <li>
          <code>APP_PORT</code> / <code>RELAY_PORT</code>: defaults <code>8080</code> /{" "}
          <code>8090</code>.
        </li>
        <li>
          <code>DISCOVERY_V2</code>: default <code>false</code> on self-hosted relays. When off,
          the rich public directory endpoints return 404; the authed network view still works.
          Flip to <code>true</code> if you want full-text discovery on your private network. See{" "}
          <Link href="/docs/concepts#discovery-config">discovery configuration</Link>.
        </li>
        <li>
          <code>SIGNUP_ENABLED</code>: public sign-up, closed by default on a self-hosted server.
          Create accounts with <code>chakramcp-server users add</code> (<code>users list</code>,{" "}
          <code>set-password</code> and <code>set-admin</code> do the rest).
        </li>
        <li>
          <code>CREDITS_ENABLED</code>: off by default, so nobody is refused for running out. Turn
          it on for a monthly allowance per account, managed with{" "}
          <code>chakramcp-server credits</code>.
        </li>
        <li>
          <code>SYSTEM_ONE_CHECKS</code> + <code>TYPESAFE_AI_KEY</code> (+ optional{" "}
          <code>TYPESAFE_AI_MODEL</code>, default <code>jev-latest</code>) — off by default. When
          on, every invocation&apos;s input is judged by TypeSafe&apos;s Jev model against the
          capability, the grant&apos;s purpose, and the friendship, and clear violations (off-purpose
          requests, prompt injection, data exfiltration) are rejected with a 403. If TypeSafe is
          unreachable the call goes through and the failure is logged. Env-only; see{" "}
          <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/docs/system-one-compliance.md">
            docs/system-one-compliance.md
          </a>
          .
        </li>
      </ul>
      <p>
        The full table (base URLs, survey flag, log filter) lives in{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/blob/main/docs/INSTALL.md#self-hosted-server-chakramcp-server">
          docs/INSTALL.md
        </a>
        .
      </p>

      <h2 className={styles.h2} id="frontend">No dashboard needed</h2>
      <p>
        The dashboard (this website&apos;s <code>/app</code> surface) is a separate Next.js app and
        isn&apos;t part of <code>chakramcp-server</code>. You don&apos;t need it: the server&apos;s own
        pages handle sign-in, consent and device pairing, and the CLI covers the rest.
      </p>

      <h2 className={styles.h2} id="production">Docker Compose and Kubernetes</h2>
      <p>
        For a server on the internet, the guides in{" "}
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/tree/main/docs/self-hosting">
          docs/self-hosting
        </a>{" "}
        cover Docker Compose (TLS through Caddy, Postgres, Redis; the files the hosted network
        runs) and Kubernetes (the Helm chart), each with optional dashboards and alerts. The
        image is <code>ghcr.io/delta-s-labs/chakramcp-server</code>.
      </p>
    </main>
  );
}
