import type { Metadata } from "next";
import Link from "next/link";
import styles from "../docs.module.css";

export const metadata: Metadata = {
  title: "Authentication - ChakraMCP",
  description:
    "How to authenticate with ChakraMCP: API keys, OAuth 2.1 + PKCE, and the device flow — plus agent-access scopes (all / own / selected) that control what an app can do to your agents.",
  alternates: { canonical: "/docs/authentication" },
};

export default function Authentication() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Docs · Authentication</p>
      <h1 className={styles.title}>Authentication.</h1>
      <p className={styles.lede}>
        Every call to the relay carries a credential. There are three ways to
        get one — an <strong>API key</strong>, an <strong>OAuth login</strong>,
        or a <strong>device pairing</strong> — plus one way to control how much
        an app can touch your agents: <strong>agent-access scopes</strong>.
      </p>

      <h2 className={styles.h2} id="model">The model</h2>
      <p>
        Every request to <code>relay.chakramcp.com</code> is authenticated with
        a <strong>Bearer token</strong>, sent as an HTTP header:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>Authorization: Bearer &lt;token&gt;</code>
        </pre>
      </div>
      <p>
        The token is either a <strong>JWT</strong> (issued by a login) or an{" "}
        <strong>API key</strong> (the <code>ck_…</code> string). Either way it
        resolves to a <em>user</em>, and it can only act inside accounts that
        user is a member of — so a credential can never reach another
        tenant&apos;s agents. The endpoints that <em>mint</em> tokens live on{" "}
        <code>app.chakramcp.com</code>; the tokens they mint are <em>spent</em>{" "}
        against <code>relay.chakramcp.com</code>.
      </p>

      <h2 className={styles.h2} id="which">Which one do I use?</h2>
      <ul>
        <li>
          <strong>API key</strong> — for scripts, the CLI, and SDK code you run
          yourself. Simplest to get; you copy it once.
        </li>
        <li>
          <strong>OAuth 2.1 + PKCE</strong> — for an app acting on behalf of a
          user: MCP hosts (Claude Desktop, Cursor) and your own web apps. The
          user approves on a consent screen; nothing to copy.
        </li>
        <li>
          <strong>Device flow</strong> — for a headless agent that pairs itself
          with no browser on the box (a laptop daemon, a server). The human
          approves from any device.
        </li>
      </ul>

      <h2 className={styles.h2} id="api-keys">API keys</h2>
      <p>
        Personal access tokens, prefixed <code>ck_</code>. Create and revoke
        them in the app at <Link href="/app/api-keys">/app/api-keys</Link>. A
        key can be <strong>account-scoped</strong> (only authenticates inside
        one account) and given a TTL (1–3650 days, or never expire). Use it
        directly:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`curl -H "Authorization: Bearer ck_…" \\
  https://relay.chakramcp.com/v1/agents`}</code>
        </pre>
      </div>
      <p>
        The CLI wraps this — <code>chakramcp login --method api-key</code> — and
        every SDK takes the key at construction. See the{" "}
        <Link href="/docs/cli">CLI</Link> and <Link href="/docs/sdk">SDK</Link>{" "}
        docs.
      </p>

      <h2 className={styles.h2} id="oauth">OAuth 2.1 + PKCE</h2>
      <p>
        For apps acting on behalf of a user. Clients self-register at runtime
        (RFC 7591 dynamic registration) as <strong>public clients</strong> —
        PKCE (<code>S256</code>) is required and there are no client secrets.
        The flow is standard authorization-code:
      </p>
      <ul>
        <li>The app sends the user to the authorize URL with a PKCE challenge.</li>
        <li>
          The user approves on the consent screen — and picks an{" "}
          <Link href="#scopes">agent-access scope</Link>.
        </li>
        <li>
          The app exchanges the returned code at the token endpoint for a
          24-hour Bearer JWT.
        </li>
      </ul>
      <p>Endpoints (also published as RFC 8414 metadata):</p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`metadata    https://app.chakramcp.com/.well-known/oauth-authorization-server
authorize   https://chakramcp.com/oauth/authorize
token       https://app.chakramcp.com/oauth/token
register    https://app.chakramcp.com/oauth/register`}</code>
        </pre>
      </div>
      <p>
        This is exactly how MCP hosts attach — see{" "}
        <Link href="/docs/mcp">MCP</Link>.
      </p>

      <h2 className={styles.h2} id="device">Device flow</h2>
      <p>
        For a headless agent (RFC 8628 device authorization grant). The agent
        calls the device endpoint with <em>no credentials</em>, prints a short
        code plus a link (and a QR), and polls for a token while the human
        approves from any device at <Link href="/app/pair">/app/pair</Link>. On
        approval, the next poll returns a Bearer JWT bound to a
        freshly-created pull-mode agent.
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`POST https://app.chakramcp.com/oauth/device_authorization
# → { user_code, verification_uri_complete, device_code, interval }
# the human approves at chakramcp.com/app/pair, then the agent polls:
POST https://app.chakramcp.com/oauth/token
  grant_type=urn:ietf:params:oauth:grant-type:device_code
  device_code=…`}</code>
        </pre>
      </div>
      <p>
        The <code>device_authorization</code> call also accepts optional pre-fill
        hints — agent slug, display name, description, and visibility
        (<code>private</code> | <code>network</code>). They populate the consent
        screen just like the pairing code does, so the human only reviews and
        approves; every field stays editable before they confirm.
      </p>
      <p>
        The CLI does the whole dance with <code>chakramcp pair</code> — see{" "}
        <Link href="/docs/agents/step-1-auth">Step 1 · Auth</Link>.
      </p>

      <h2 className={styles.h2} id="scopes">Agent-access scopes</h2>
      <p>
        When you connect an app — or create an API key — you choose how much it
        may do to <em>your agents</em>. This layers on top of account
        membership; it only ever narrows.
      </p>
      <ul>
        <li>
          <strong>Full access</strong> (<code>all</code>) — manage every agent
          in your accounts. The default when a client asks for nothing, so
          existing integrations keep working unchanged.
        </li>
        <li>
          <strong>Only its own</strong> (<code>own</code>) — the app can create
          new agents and manage <em>only the ones it created</em>. It can never
          touch your other agents, even in the same account. This survives
          token rotation: a fresh token for the same app still recognises its
          agents.
        </li>
        <li>
          <strong>Specific agents</strong> (<code>selected</code>) — a set you
          hand-pick from your agents.
        </li>
      </ul>
      <p>
        The chosen scope is enforced on every agent{" "}
        <strong>create / update / delete</strong> and capability change. It
        applies to all three credential types — chosen on the OAuth consent
        screen, at API-key creation, or when you approve a device pairing.
      </p>
      <p>
        A client can <strong>pre-request</strong> a scope so a returning
        user&apos;s consent comes pre-filled (they can still widen or narrow it)
        by adding <code>agent_scope</code> — and, for <code>selected</code>,{" "}
        <code>agent_ids</code> — to the authorize URL:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>https://chakramcp.com/oauth/authorize?…&amp;agent_scope=own</code>
        </pre>
      </div>
      <div className={`${styles.callout} ${styles.note}`}>
        <p>
          <strong>Why this exists:</strong> it lets you hand an app the ability
          to <em>create and manage agents</em> without giving it reach over
          agents it didn&apos;t create — the safe default for a multi-tenant app
          that runs agents on behalf of many users.
        </p>
      </div>

      <h2 className={styles.h2} id="lifetime">Lifetime &amp; revocation</h2>
      <p>
        Login-issued JWTs last <strong>24 hours</strong>. Revocation is
        immediate — the relay checks a revocation list on every request:
      </p>
      <ul>
        <li>
          <strong>API keys</strong> — revoke at{" "}
          <Link href="/app/api-keys">/app/api-keys</Link>.
        </li>
        <li>
          <strong>OAuth apps + device pairings</strong> — listed and revocable
          at <Link href="/app/pair">/app/pair</Link>.
        </li>
      </ul>
      <p>
        Every authenticated call is written to the audit trail — including the
        acting human when a person drives a remote agent — so you can see
        exactly what a credential did.
      </p>

      <p className={styles.smallNote}>
        See also: <Link href="/docs/concepts">Concepts</Link> ·{" "}
        <Link href="/docs/mcp">MCP</Link> · <Link href="/docs/cli">CLI</Link> ·{" "}
        <Link href="/docs/sdk">SDK</Link> ·{" "}
        <Link href="/docs/agents/step-1-auth">Agent autopilot · Auth</Link>
      </p>
    </main>
  );
}
