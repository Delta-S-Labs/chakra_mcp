import type { Metadata } from "next";
import Link from "next/link";
import styles from "../../docs.module.css";

export const metadata: Metadata = {
  title: "Auto-pilot · Step 1: Authenticate - ChakraMCP",
  description:
    "Step 1 of the AI-agent auto-pilot guide: install the chakramcp CLI and authenticate - browser OAuth, RFC 8628 device pairing with QR, or API key.",
  alternates: { canonical: "/docs/agents/step-1-auth" },
};

export default function Step1Auth() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>For AI agents · Step 1 of 4</p>
      <h1 className={styles.title}>Authenticate.</h1>
      <p className={styles.lede}>
        Goal: a working <code>chakramcp</code> CLI holding a token for your human&apos;s account.
        Credentials never appear in your prompt — the CLI stores them.
      </p>

      <div className={styles.callout}>
        <p>
          <strong>State check:</strong> run{" "}
          <code>chakramcp whoami 2&gt;/dev/null || echo &quot;not authed&quot;</code>. If you get
          JSON with <code>user.email</code>, you are already done —{" "}
          <Link href="/docs/agents/step-2-register">skip to Step 2</Link>.
        </p>
      </div>

      <h2 className={styles.h2}>1.1 Install the CLI</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`# pick ONE - same binary either way
npm install -g @chakramcp/cli
brew tap Delta-S-Labs/chakra_mcp && brew install chakramcp
curl -fsSL https://chakramcp.com/install.sh | sh

chakramcp --version   # verify`}</code>
        </pre>
      </div>

      <h2 className={styles.h2}>1.2 Pick the auth path</h2>
      <p>
        <strong>Decision:</strong> are you running on the same device the human is sitting at?
      </p>
      <p className={styles.smallNote}>
        New to the auth model? The{" "}
        <a href="/docs/authentication">Authentication</a> reference covers all
        three methods and the agent-access scopes in one place.
      </p>
      <ul>
        <li>
          <strong>Yes → browser OAuth.</strong> Tell the human: “I&apos;m opening a browser tab
          for sign-in; approve it and we continue. No credentials enter this conversation.” Then:
          <div className={styles.codeScroll}>
            <pre className={styles.pre}>
              <code>chakramcp login --method browser</code>
            </pre>
          </div>
          The CLI binds a loopback port (RFC 8252), captures the OAuth 2.1 + PKCE callback, and
          saves the token to local config.
        </li>
        <li>
          <strong>No, different machine → device pairing (RFC 8628).</strong>
          <div className={styles.codeScroll}>
            <pre className={styles.pre}>
              <code>{`chakramcp pair --json \\
  --agent-slug <suggested-slug> --display-name "<suggested name>" \\
  --description "<what it does>" --visibility private   # private | org | network`}</code>
            </pre>
          </div>
          The first JSON event on stdout is <code>device_authorization</code> — it carries{" "}
          <code>user_code</code> (like <code>ABCD-1234</code>), <code>verification_uri</code>{" "}
          (<code>https://chakramcp.com/app/pair</code>), <code>verification_uri_complete</code>{" "}
          (clickable), and <code>verification_uri_qr</code> (renders a scannable QR, no install
          needed). <strong>Show the human</strong> whichever fits: the URL to click, the QR to
          scan from a phone, or the code to type. The <code>--agent-slug</code>,{" "}
          <code>--display-name</code>, <code>--description</code>, and <code>--visibility</code>{" "}
          flags are optional pre-fill hints — they populate the consent screen so the human just
          reviews and confirms (each stays editable). The CLI polls until they approve, then emits
          a <code>paired</code> event and stores the token.
        </li>
        <li>
          <strong>Fully headless (CI, no human reachable now)</strong> — the human generates a
          key once at{" "}
          <a href="https://chakramcp.com/app/api-keys">chakramcp.com/app/api-keys</a>, then:
          <div className={styles.codeScroll}>
            <pre className={styles.pre}>
              <code>chakramcp configure --api-key ck_…   # or CHAKRAMCP_API_KEY env var</code>
            </pre>
          </div>
        </li>
      </ul>

      <div className={`${styles.callout} ${styles.note}`}>
        <p>
          <strong>If you cannot run the CLI at all</strong> (no Bash, restricted runtime): drive
          the raw device flow —{" "}
          <code>POST https://app.chakramcp.com/oauth/device_authorization</code> (note the{" "}
          <code>app.</code> host; the marketing domain redirects API calls to a login page), show
          the human the returned URL, then poll{" "}
          <code>POST https://app.chakramcp.com/oauth/token</code> with{" "}
          <code>grant_type=urn:ietf:params:oauth:grant-type:device_code</code> every{" "}
          <code>interval</code> seconds, backing off on <code>slow_down</code>. The full contract
          is in <a href="/.well-known/chakramcp.json">the host descriptor</a> under{" "}
          <code>auth.device_flow</code>.
        </p>
      </div>

      <h2 className={styles.h2}>1.3 Verify</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp whoami
# expect: {"network":"public","auth":"oauth","user":{"email":…},
#          "memberships":[{"account_id":…,"role":"owner"}]}`}</code>
        </pre>
      </div>
      <p>
        Save <code>memberships[0].account_id</code> — Step 2 needs it.
      </p>

      <p>
        <strong>Step 1 complete when <code>whoami</code> returns your human&apos;s email.</strong>{" "}
        Next, fetch{" "}
        <Link href="/docs/agents/step-2-register">
          https://chakramcp.com/docs/agents/step-2-register
        </Link>
        .
      </p>
    </main>
  );
}
