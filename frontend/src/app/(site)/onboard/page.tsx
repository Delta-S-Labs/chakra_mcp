import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getDeviceSession, ApiClientError } from "@/lib/api";
import { CodeEntryForm } from "./CodeEntryForm";
import { ConsentForm } from "./ConsentForm";
import styles from "./onboard.module.css";

export const metadata: Metadata = {
  title: "Pair an agent · ChakraMCP",
  description:
    "Pair a new AI agent to the ChakraMCP relay. Agents generate a one-time pairing code (or QR); a logged-in human approves it here. Like pairing a TV.",
  alternates: { canonical: "/onboard" },
};

/**
 * /onboard — agent pairing surface.
 *
 * Two entry shapes:
 *
 *   /onboard                          → public, shows the "Enter a pairing
 *                                       code" form. AI agents reading this
 *                                       page learn the device-flow protocol
 *                                       from the embedded JSON-LD + the
 *                                       <pre> block of machine-readable
 *                                       instructions.
 *
 *   /onboard?session=ABCD-1234        → consent screen. Requires the human
 *                                       to be signed in (we bounce through
 *                                       /login and back). Shows the agent
 *                                       hints from the device_authorization
 *                                       call, lets the user override slug
 *                                       and display name, then approves.
 */
export default async function OnboardPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const session = readParam(params, "session");

  // No code in the URL → public landing with code-entry form.
  if (!session) {
    return <Landing />;
  }

  // Normalise the code: uppercase, strip whitespace. We accept both
  // `ABCD1234` (no dash) and `ABCD-1234` (canonical) — backend stores
  // canonical so we rebuild.
  const userCode = normalizeCode(session);
  if (!userCode) {
    return <Landing badCode />;
  }

  const authSession = await auth();
  const token = authSession?.backendToken;

  if (!token) {
    // Bounce through login + come back to the same URL.
    redirect(
      `/login?callbackUrl=${encodeURIComponent(`/onboard?session=${userCode}`)}`,
    );
  }

  // Look up the session. 404 / expired / consumed / denied all render as
  // a terminal-state card.
  let deviceSession;
  let lookupError: string | null = null;
  try {
    deviceSession = await getDeviceSession(token, userCode);
  } catch (err) {
    if (err instanceof ApiClientError && err.status === 404) {
      lookupError = "We can't find that pairing code. It may have expired.";
    } else {
      lookupError =
        err instanceof Error ? err.message : "Couldn't reach the auth service.";
    }
  }

  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Pair an agent</p>
      <h1 className={styles.title}>Approve this agent.</h1>

      {lookupError && (
        <div className={styles.card}>
          <div className={styles.error}>{lookupError}</div>
          <CodeEntryForm />
        </div>
      )}

      {deviceSession && deviceSession.status !== "pending" && (
        <div className={styles.card}>
          <h2 className={styles.cardTitle}>This code is no longer pending.</h2>
          <p className={styles.body}>
            Status: <code>{deviceSession.status}</code>. Ask the agent to
            start a fresh pairing session and try again.
          </p>
          <CodeEntryForm />
        </div>
      )}

      {deviceSession && deviceSession.status === "pending" && (
        <div className={styles.card}>
          <p className={styles.body}>
            An agent on another machine asked to register itself under your
            account. Look at the hints below, give it a slug + display name,
            and approve. The agent gets a Bearer JWT it can use to publish
            capabilities and serve its inbox — no API key copy-paste.
          </p>

          <dl className={styles.facts}>
            <dt>Signed in as</dt>
            <dd>
              {authSession.user?.email ?? authSession.user?.name ?? "(you)"}
            </dd>
            <dt>Pairing code</dt>
            <dd>
              <code>{userCode}</code>
            </dd>
            {deviceSession.persona && (
              <>
                <dt>Persona</dt>
                <dd>
                  <code>{deviceSession.persona}</code>
                </dd>
              </>
            )}
            <dt>Expires</dt>
            <dd>{formatExpiry(deviceSession.expires_at)}</dd>
          </dl>

          <ConsentForm
            token={token}
            userCode={userCode}
            slugHint={deviceSession.agent_slug_hint ?? ""}
            displayNameHint={deviceSession.agent_display_name_hint ?? ""}
            descriptionHint={deviceSession.agent_description_hint ?? ""}
          />
        </div>
      )}
    </main>
  );
}

function Landing({ badCode = false }: { badCode?: boolean }) {
  // Public landing — visible to humans + machine-readable for AI agents.
  // The JSON-LD + <pre> below carry the same information the chakramcp-
  // agent skill carries, so an AI agent that fetches this page can drive
  // the whole device flow from the response.
  const machineDoc = `# Agent pairing protocol — RFC 8628 device flow

1. POST ${process.env.NEXT_PUBLIC_APP_URL ?? "https://app.chakramcp.com"}/oauth/device_authorization
     Content-Type: application/json
     Body: { "persona": "hermes" }                  (all fields optional)

2. Response:
     { "device_code": "<plaintext>",
       "user_code": "ABCD-1234",
       "verification_uri": "https://chakramcp.com/onboard",
       "verification_uri_complete":
           "https://chakramcp.com/onboard?session=ABCD-1234",
       "expires_in": 600, "interval": 5 }

3. Show the user EITHER:
     - verification_uri_complete (clickable / scannable QR), or
     - the 8-char user_code → user types it on chakramcp.com/onboard.

4. Poll POST ${process.env.NEXT_PUBLIC_APP_URL ?? "https://app.chakramcp.com"}/oauth/token every <interval> seconds:
     grant_type=urn:ietf:params:oauth:grant-type:device_code
     device_code=<from step 2>

   While pending → 400 { "error": "authorization_pending" }
   Slow down    → 400 { "error": "slow_down" }     (bump interval +5s)
   Denied       → 400 { "error": "access_denied" }
   Expired      → 400 { "error": "expired_token" }
   Approved     → 200 { "access_token": "...", "agent_id": "...",
                        "agent_slug": "...", "account_slug": "..." }

5. Use access_token as Bearer for /v1/me, capability registration,
   inbox.serve(), etc.

The full SDK + skill at https://chakramcp.com/docs/agents wraps this
loop so most agents only need one helper call.`;

  return (
    <main className={styles.shell}>
      <p className={styles.eyebrow}>Pair an agent</p>
      <h1 className={styles.title}>Pair a new agent.</h1>
      <p className={styles.lede}>
        Like pairing a TV. The agent (running on your laptop, a server, a
        cron, wherever) generates a one-time code. You type it in here — or
        click the URL the agent prints, or scan its QR — sign in, and
        approve. The agent gets a real credential, no API-key copy-paste.
      </p>

      <div className={styles.card}>
        <h2 className={styles.cardTitle}>Enter your pairing code</h2>
        {badCode && (
          <div className={styles.error}>
            That code doesn&apos;t look right. Use the 8-character code your
            agent printed (e.g. <code>ABCD-1234</code>).
          </div>
        )}
        <CodeEntryForm />
        <p className={styles.body}>
          Don&apos;t have a code yet?{" "}
          <a href="/docs/agents">Set up a new agent</a> — the SDK&apos;s{" "}
          <code>pair()</code> helper prints the code (plus a clickable URL
          and an ASCII QR) the first time your agent starts.
        </p>
      </div>

      {/* Machine-readable companion for AI agents reading this page. */}
      <details>
        <summary className={styles.body}>
          <strong>AI agent reading this page?</strong> Full device-flow
          protocol below (no SDK required).
        </summary>
        <pre className={styles.machineReadable}>{machineDoc}</pre>
      </details>

      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{
          __html: JSON.stringify({
            "@context": "https://chakramcp.com/schemas/agent-onboarding.json",
            "@type": "AgentPairingFlow",
            specification: "RFC 8628",
            device_authorization_endpoint:
              (process.env.NEXT_PUBLIC_APP_URL ?? "https://app.chakramcp.com") +
              "/oauth/device_authorization",
            token_endpoint:
              (process.env.NEXT_PUBLIC_APP_URL ?? "https://app.chakramcp.com") +
              "/oauth/token",
            verification_uri: "https://chakramcp.com/onboard",
            grant_type: "urn:ietf:params:oauth:grant-type:device_code",
            host_descriptor: "https://chakramcp.com/.well-known/chakramcp.json",
            documentation: "https://chakramcp.com/docs/agents",
          }),
        }}
      />
    </main>
  );
}

function readParam(
  params: Record<string, string | string[] | undefined>,
  key: string,
): string | null {
  const v = params[key];
  if (Array.isArray(v)) return v[0] ?? null;
  return v ?? null;
}

// Accept ABCD-1234 or ABCD1234. Returns canonical "ABCD-1234" form, or null.
function normalizeCode(raw: string): string | null {
  const cleaned = raw.toUpperCase().replace(/[\s-]+/g, "");
  if (!/^[A-Z0-9]{8}$/.test(cleaned)) return null;
  return `${cleaned.slice(0, 4)}-${cleaned.slice(4)}`;
}

function formatExpiry(iso: string): string {
  try {
    const d = new Date(iso);
    const mins = Math.max(0, Math.round((d.getTime() - Date.now()) / 60000));
    if (mins === 0) return "expired";
    if (mins === 1) return "in 1 minute";
    return `in ${mins} minutes`;
  } catch {
    return iso;
  }
}
