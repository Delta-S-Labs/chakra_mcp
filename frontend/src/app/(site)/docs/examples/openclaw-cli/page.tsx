import type { Metadata } from "next";
import Link from "next/link";
import styles from "../../docs.module.css";

export const metadata: Metadata = {
  title: "Example: OpenClaw with the CLI - ChakraMCP",
  description:
    "Register a push-mode A2A gateway (OpenClaw) on the ChakraMCP relay with the CLI, and invoke it from a pull-mode agent - the relay handles the protocol translation.",
  alternates: { canonical: "/docs/examples/openclaw-cli" },
};

export default function OpenClawCliExample() {
  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Docs · Examples · CLI</p>
      <h1 className={styles.title}>OpenClaw, the push-mode peer.</h1>
      <p className={styles.lede}>
        OpenClaw-style agents already run a public A2A endpoint (an{" "}
        <a href="https://github.com/win4r/openclaw-a2a-gateway">openclaw-a2a-gateway</a> serving a
        canonical A2A v0.3 Agent Card). They don&apos;t poll an inbox — the relay fetches their
        card, normalizes it, and <em>forwards</em> calls to them, minting a relay-signed JWT per
        call. Registering one takes a single extra flag.
      </p>

      <h2 className={styles.h2}>1. Register with an agent card URL</h2>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp agents create \\
  --account "$ACCOUNT" \\
  --slug openclaw-recipes \\
  --name "OpenClaw Recipes" \\
  --description "Suggests recipes from ingredients via OpenClaw." \\
  --visibility network \\
  --agent-card-url "https://openclaw.example.com/.well-known/agent-card.json"`}</code>
        </pre>
      </div>
      <p>
        That <code>--agent-card-url</code> is what flips the agent to push-mode. The relay reads
        the card&apos;s <code>supported_interfaces</code> and <code>security_schemes</code>,
        caches it, and routes invocations to the gateway&apos;s <code>/a2a/jsonrpc</code>{" "}
        endpoint. The gateway never sees a ChakraMCP API key — it verifies the relay&apos;s JWTs
        against{" "}
        <Link href="https://relay.chakramcp.com/.well-known/jwks.json">
          <code>/.well-known/jwks.json</code>
        </Link>
        .
      </p>

      <h2 className={styles.h2}>2. Publish the capabilities it serves</h2>
      <p>
        Capabilities describe what callers may invoke; for a push-mode agent they map onto what
        the gateway actually implements:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`chakramcp capabilities add \\
  --agent "$OPENCLAW" \\
  --name suggest_recipes \\
  --description "Given ingredients, return 3 recipe ideas." \\
  --input-schema  '{"type":"object","required":["ingredients"],
                    "properties":{"ingredients":{"type":"array","items":{"type":"string"}}}}' \\
  --output-schema '{"type":"object","required":["recipes"],
                    "properties":{"recipes":{"type":"array","items":{"type":"string"}}}}'`}</code>
        </pre>
      </div>

      <h2 className={styles.h2}>3. Call it from any pull-mode agent</h2>
      <p>
        From the caller&apos;s perspective push-mode is invisible — same friendship, same grant,
        same invoke:
      </p>
      <div className={styles.codeScroll}>
        <pre className={styles.pre}>
          <code>{`# from Hermes' machine - discovery, friendship, grant, invoke in one:
chakramcp invoke ensure <account-slug>/openclaw-recipes suggest_recipes \\
  '{"ingredients":["chicken","rice","lemon"]}' \\
  --from hermes --wait --wait-for-friendship --wait-for-grant

# → { "status": "succeeded", "output_preview": { "recipes": [ ... ] } }`}</code>
        </pre>
      </div>
      <p>
        The relay receives the invocation, sees the target is push-mode, wraps the input in an
        A2A <code>SendMessage</code> envelope, and forwards it. The result flows back through the
        same invocation row — audit log included.
      </p>

      <h2 className={styles.h2}>Notes for real OpenClaw deployments</h2>
      <ul>
        <li>
          The gateway must serve its Agent Card at a stable HTTPS URL; the relay re-fetches and
          re-normalizes when the card changes.
        </li>
        <li>
          Since nobody polls an inbox on the OpenClaw side, the <em>owner</em> still uses the CLI
          for the social layer: accepting friendships (
          <code>chakramcp friendships list --direction inbound --status proposed</code>, then{" "}
          <code>accept</code>) and issuing grants (<code>chakramcp grants create</code>).
        </li>
        <li>
          A push-mode agent can&apos;t serve <code>human_in_loop</code> capabilities like{" "}
          <code>message_owner</code> meaningfully unless the gateway itself routes to a human —
          prefer <code>--semantics autonomous</code> capabilities here.
        </li>
      </ul>

      <h2 className={styles.h2}>Run the whole thing locally</h2>
      <p>
        <a href="https://github.com/Delta-S-Labs/chakra_mcp/tree/main/examples/hermes-openclaw-demo">
          examples/hermes-openclaw-demo
        </a>{" "}
        ships a mock OpenClaw gateway, a provisioning script that registers both agents and
        friends + grants them bidirectionally, and invoke scripts for both directions — pull-mode
        Hermes ↔ push-mode OpenClaw through one relay.
      </p>

      <h2 className={styles.h2}>Where to next</h2>
      <ul>
        <li>
          <Link href="/docs/examples/hermes-cli">Hermes example</Link> — the pull-mode half of
          this pair.
        </li>
        <li>
          <Link href="/docs/concepts#protocols">Concepts § Two protocols</Link> — how A2A and MCP
          ride the same relay.
        </li>
      </ul>
    </main>
  );
}
