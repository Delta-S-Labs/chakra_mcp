import { redirect } from "next/navigation";
import { auth } from "@/auth";
import { getOAuthClient, ApiClientError } from "@/lib/api";
import { ConsentForm } from "./ConsentForm";
import styles from "./oauth.module.css";

/**
 * /oauth/authorize — OAuth 2.1 + PKCE consent screen.
 *
 * The MCP client redirected the user's browser here with the standard
 * authorization-request query parameters. We:
 *   1. Verify NextAuth session (proxy.ts redirects to /login otherwise).
 *   2. Look up the registered client to show its real display name.
 *   3. Validate that redirect_uri is one of the client's registered URIs.
 *   4. Render the consent UI; on approve the client component asks the
 *      backend to mint an auth code and bounces the browser to the
 *      client's redirect_uri.
 */
export default async function OAuthAuthorizePage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const session = await auth();
  const token = session?.backendToken;
  if (!token) redirect("/login");

  const responseType = readParam(params, "response_type");
  const clientId = readParam(params, "client_id");
  const redirectUri = readParam(params, "redirect_uri");
  const codeChallenge = readParam(params, "code_challenge");
  const codeChallengeMethod = readParam(params, "code_challenge_method") ?? "S256";
  const state = readParam(params, "state") ?? "";
  const scope = readParam(params, "scope") ?? "relay.full";

  const errors: string[] = [];
  if (responseType !== "code") errors.push("response_type must be 'code'.");
  if (!clientId) errors.push("client_id is required.");
  if (!redirectUri) errors.push("redirect_uri is required.");
  if (!codeChallenge) errors.push("code_challenge is required (PKCE).");
  if (codeChallengeMethod !== "S256") errors.push("code_challenge_method must be 'S256'.");

  let client = null;
  let clientLookupError: string | null = null;
  if (errors.length === 0 && clientId) {
    try {
      client = await getOAuthClient(clientId);
    } catch (err) {
      if (err instanceof ApiClientError && err.status === 404) {
        clientLookupError = "This MCP client isn't registered with us.";
      } else {
        clientLookupError =
          err instanceof Error ? err.message : "Couldn't reach the auth service.";
      }
    }
  }
  if (client && redirectUri && !client.redirect_uris.includes(redirectUri)) {
    errors.push("redirect_uri does not match any URI registered for this client.");
  }

  return (
    <main className={styles.shell}>
      <div className={styles.card}>
        <div className="eyebrow">Authorize</div>
        <h1 className={styles.title}>
          {client ? client.client_name : "An MCP client"} wants to act on your behalf.
        </h1>
        <p className={styles.body}>
          Approving will let this app call <strong>any of your relay tools</strong>
          {" "}— invoke capabilities you&apos;ve been granted, pull pending work
          from your inbox, propose friendships, and read your audit log. Tokens
          last 24 hours; revoke anytime by signing the client back out.
        </p>

        {client?.client_uri && (
          <p className={styles.body}>
            Client homepage:{" "}
            <a className={styles.link} href={client.client_uri} target="_blank" rel="noreferrer">
              {client.client_uri}
            </a>
          </p>
        )}

        <dl className={styles.facts}>
          <dt>Signed in as</dt>
          <dd>{session?.user?.email ?? session?.user?.name ?? "—"}</dd>
          <dt>Scope</dt>
          <dd><code>{scope}</code></dd>
          <dt>Redirect</dt>
          <dd>
            <code className={styles.redirect}>{redirectUri}</code>
          </dd>
        </dl>

        {clientLookupError && <div className={styles.error}>{clientLookupError}</div>}
        {errors.map((e) => (
          <div key={e} className={styles.error}>{e}</div>
        ))}

        {errors.length === 0 && client && clientId && redirectUri && codeChallenge && (
          <ConsentForm
            token={token}
            clientId={clientId}
            redirectUri={redirectUri}
            codeChallenge={codeChallenge}
            codeChallengeMethod="S256"
            state={state}
            scope={scope}
          />
        )}
      </div>
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
