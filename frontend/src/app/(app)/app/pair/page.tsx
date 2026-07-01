import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { auth } from "@/auth";
import {
  ApiClientError,
  getDeviceSession,
  getUsageSummary,
  listPairings,
  type Pairing,
  type UsagePairRollup,
} from "@/lib/api";
import { listMyAgents } from "@/lib/relay";
import { CodeEntryForm } from "./CodeEntryForm";
import { ConsentForm, type PairableAgent } from "./ConsentForm";
import { PairingsList } from "./PairingsList";
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

  // No code in the URL → show the code-entry form + the paired
  // agents list.
  if (!session) {
    let pairings: Pairing[] = [];
    let pairUsage: UsagePairRollup[] = [];
    let pairingsError: string | null = null;
    if (token) {
      try {
        // Fetch both in parallel. Usage is a soft dependency — if it
        // fails we still render the list, just without the per-row
        // request counts.
        const [pairingsResult, usageResult] = await Promise.allSettled([
          listPairings(token),
          getUsageSummary(token),
        ]);
        if (pairingsResult.status === "fulfilled") {
          pairings = pairingsResult.value;
        } else {
          pairingsError =
            pairingsResult.reason instanceof Error
              ? pairingsResult.reason.message
              : "Couldn't load paired agents.";
        }
        if (usageResult.status === "fulfilled") {
          pairUsage = usageResult.value.by_pair;
        }
      } catch (err) {
        pairingsError =
          err instanceof Error ? err.message : "Couldn't load paired agents.";
      }
    }
    return (
      <Landing
        pairings={pairings}
        pairUsage={pairUsage}
        pairingsError={pairingsError}
        token={token ?? null}
      />
    );
  }

  // Normalise the code: uppercase, strip whitespace. Accept both
  // `ABCD1234` (no dash) and `ABCD-1234` (canonical) — backend stores
  // canonical so we rebuild.
  const userCode = normalizeCode(session);
  if (!userCode) {
    return <Landing badCode />;
  }

  // Build the return URL up front so any auth bounce preserves the
  // pairing code — after sign-in the user resumes here on the approve
  // screen, not a bare /app/pair (or worse, an "unauthorized" dead-end).
  const fromUrl = `/app/pair?session=${encodeURIComponent(userCode)}`;

  // No usable token. The NextAuth cookie can outlive the backend JWT it
  // carries (see auth.ts), so `proxy.ts` + the app layout happily let a
  // cookie-alive-but-token-dead visitor reach here. Bounce to /login,
  // preserving the code, instead of rendering a broken screen.
  if (!token) {
    redirect(`/login?from=${encodeURIComponent(fromUrl)}`);
  }

  // Look up the session. 404 / expired / consumed / denied all render as
  // a terminal-state card; a 401 means our own token is stale/dead — send
  // the user through /login (code preserved) rather than showing the raw
  // "Unauthorized" the backend returns.
  let deviceSession;
  let lookupError: string | null = null;
  let sessionExpired = false;
  try {
    deviceSession = await getDeviceSession(token, userCode);
  } catch (err) {
    if (err instanceof ApiClientError && err.status === 404) {
      lookupError = "We can't find that pairing code. It may have expired.";
    } else if (
      (err instanceof ApiClientError && err.status === 401) ||
      (err instanceof Error && /\b401\b|unauthoriz/i.test(err.message))
    ) {
      // Stale/dead backend token: a valid NextAuth cookie can still carry an
      // expired JWT (see auth.ts), so we arrive here "logged in" while the
      // authed device-session lookup 401s. Detect it by status OR message
      // (mirrors /app/page.tsx's isUnauthorized) and bounce to login with
      // the code preserved, rather than rendering the raw "unauthorized".
      sessionExpired = true;
    } else {
      lookupError =
        err instanceof Error ? err.message : "Couldn't reach the auth service.";
    }
  }
  // redirect() throws NEXT_REDIRECT, so it must run OUTSIDE the try/catch
  // above — otherwise the catch would swallow the control-flow throw.
  if (sessionExpired) {
    redirect(`/login?from=${encodeURIComponent(fromUrl)}&reason=session_expired`);
  }

  // Agents the user already owns — offered as "pair an existing agent"
  // alternatives to creating a fresh record. Soft dependency: if it
  // fails we just fall back to create-only.
  let myAgents: PairableAgent[] = [];
  if (deviceSession && deviceSession.status === "pending") {
    try {
      const agents = await listMyAgents(token);
      myAgents = agents.map((a) => ({
        id: a.id,
        slug: a.slug,
        display_name: a.display_name,
        account_slug: a.account_slug,
      }));
    } catch {
      /* fall back to create-only */
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
            visibilityHint={deviceSession.agent_visibility_hint ?? ""}
            existingAgents={myAgents}
          />
        </div>
      )}
    </div>
  );
}

function Landing({
  badCode = false,
  pairings = [],
  pairUsage = [],
  pairingsError = null,
  token = null,
}: {
  badCode?: boolean;
  pairings?: Pairing[];
  pairUsage?: UsagePairRollup[];
  pairingsError?: string | null;
  token?: string | null;
}) {
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

      <div className={styles.codeCard}>
        <h2 className={styles.cardTitle}>Enter your pairing code</h2>
        {badCode && (
          <div className={styles.error}>
            That code doesn&apos;t look right. Use the 8-character code your
            agent printed (e.g. <code>ABCD-1234</code>).
          </div>
        )}
        <CodeEntryForm />
        <p className={styles.codeCardFoot}>
          Don&apos;t have a code yet?{" "}
          <a href="/docs/agents">Set up a new agent</a> — running{" "}
          <code>chakramcp pair</code> prints the code (plus a clickable URL
          and a QR) the first time your agent starts.
        </p>
      </div>

      <section className={styles.pairSection}>
        <div className={styles.pairHead}>
          <h2 className={styles.sectionTitle}>Paired agents</h2>
          <details className={styles.pairHint}>
            <summary aria-label="What appears in this list?">?</summary>
            <div className={styles.pairHintBody}>
              This list mixes every credential pathway you have on file:
              device-flow pairings (QR or code), browser OAuth sessions, and
              API keys. They all revoke through the same surface.
            </div>
          </details>
        </div>

        {pairingsError && <p className={styles.error}>{pairingsError}</p>}

        {!pairingsError && token && (
          <PairingsList
            initial={pairings}
            pairUsage={pairUsage}
            token={token}
          />
        )}
      </section>
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
