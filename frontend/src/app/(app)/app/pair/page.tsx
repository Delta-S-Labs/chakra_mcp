import type { Metadata } from "next";
import { auth } from "@/auth";
import { getDeviceSession, ApiClientError } from "@/lib/api";
import { CodeEntryForm } from "./CodeEntryForm";
import { ConsentForm } from "./ConsentForm";
import styles from "./pair.module.css";

export const metadata: Metadata = {
  title: "Pair an agent · ChakraMCP",
  description:
    "Pair a new AI agent to your ChakraMCP account. Agents generate a one-time pairing code; you approve it here.",
};

/**
 * /app/pair — authed agent pairing surface.
 *
 * Two entry shapes:
 *
 *   /app/pair                       → code-entry form for the signed-in
 *                                     human to type a code their agent
 *                                     printed.
 *
 *   /app/pair?session=ABCD-1234     → consent screen for an in-flight
 *                                     device-flow session. Shows the
 *                                     agent hints from the
 *                                     device_authorization call, lets
 *                                     the user override slug + display
 *                                     name, then approves.
 *
 * The parent `(app)/app/layout.tsx` already enforces auth — if the
 * user isn't signed in, middleware bounces them through `/login`
 * before this component ever renders. No fallback redirect here.
 */
export default async function PairPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const session = readParam(params, "session");
  const authSession = await auth();
  const token = authSession?.backendToken;

  // No code in the URL → show the code-entry form.
  if (!session) {
    return <Landing />;
  }

  // Normalise the code: uppercase, strip whitespace. Accept both
  // `ABCD1234` (no dash) and `ABCD-1234` (canonical) — backend stores
  // canonical so we rebuild.
  const userCode = normalizeCode(session);
  if (!userCode) {
    return <Landing badCode />;
  }

  // Defensive: layout already guarantees a session, but the typed
  // shape allows undefined.
  if (!token) {
    return <Landing />;
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
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Pair an agent</div>
        <h1 className={styles.title}>Approve this agent.</h1>
      </header>

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
              {authSession?.user?.email ?? authSession?.user?.name ?? "(you)"}
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
    </div>
  );
}

function Landing({ badCode = false }: { badCode?: boolean }) {
  return (
    <div className={styles.page}>
      <header className={styles.head}>
        <div className="eyebrow">Pair an agent</div>
        <h1 className={styles.title}>Pair a new agent.</h1>
        <p className={styles.lede}>
          Like pairing a TV. The agent (running on your laptop, a server, a
          cron, wherever) generates a one-time code. Type it in here — or
          click the URL the agent prints — and approve. The agent gets a
          real credential, no API-key copy-paste.
        </p>
      </header>

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
    </div>
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
