"use client";

import { useState } from "react";
import { issueOAuthCode, ApiClientError } from "@/lib/api";
import styles from "./oauth.module.css";

export function ConsentForm({
  token,
  clientId,
  redirectUri,
  codeChallenge,
  codeChallengeMethod,
  state,
  scope,
}: {
  token: string;
  clientId: string;
  redirectUri: string;
  codeChallenge: string;
  codeChallengeMethod: "S256";
  state: string;
  scope: string;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleApprove() {
    setError(null);
    setPending(true);
    try {
      const { code } = await issueOAuthCode(token, {
        client_id: clientId,
        redirect_uri: redirectUri,
        code_challenge: codeChallenge,
        code_challenge_method: codeChallengeMethod,
        scope,
      });
      window.location.href = appendQuery(redirectUri, { code, state });
    } catch (err) {
      // A 401 here means the NextAuth session is still present but its
      // backend token has gone stale — the proxy can't catch that (it
      // only sees "logged in"), so the consent renders, then Approve
      // dead-ends with "unauthorized". Bounce through /login (which
      // mints a fresh token) and return to this exact authorize URL via
      // the app's standard `from` param, so the OAuth flow resumes.
      if (err instanceof ApiClientError && err.status === 401) {
        const back = window.location.pathname + window.location.search;
        window.location.href = `/login?from=${encodeURIComponent(back)}`;
        return;
      }
      setError(err instanceof Error ? err.message : "Couldn't issue code.");
      setPending(false);
    }
  }

  function handleDeny() {
    window.location.href = appendQuery(redirectUri, {
      error: "access_denied",
      error_description: "User denied the consent request.",
      state,
    });
  }

  return (
    <>
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.approve}
          disabled={pending}
          onClick={handleApprove}
        >
          {pending ? "Approving…" : "Approve"}
        </button>
        <button
          type="button"
          className={styles.deny}
          disabled={pending}
          onClick={handleDeny}
        >
          Deny
        </button>
      </div>
      {error && <div className={styles.error}>{error}</div>}
    </>
  );
}

function appendQuery(uri: string, params: Record<string, string>): string {
  const url = new URL(uri);
  for (const [k, v] of Object.entries(params)) {
    if (v) url.searchParams.set(k, v);
  }
  return url.toString();
}
