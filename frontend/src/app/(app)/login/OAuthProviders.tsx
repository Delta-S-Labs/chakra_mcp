"use client";

// Shared OAuth panel used on /login + /signup. The two pages used to
// disagree about which providers were available — login had GitHub +
// Google buttons, signup didn't, even though signup's body copy
// promised them. Centralizing here keeps the surfaces consistent.
//
// NB: these buttons are deliberately NOT captcha-gated. The captcha
// only protects the email/password endpoint (where its token is
// actually verified server-side); for OAuth the token is never sent,
// and authentication happens entirely at github.com / accounts.google.com,
// which run their own bot defenses. Greying these out behind the
// captcha was pure friction with zero security value, so the gate now
// lives only on the email/password submit in the parent.

import { useState } from "react";
import { signIn } from "next-auth/react";
import styles from "./login.module.css";

type Provider = "github" | "google";

export function OAuthProviders({
  redirectTo,
  showSwitchAccountHint = false,
}: {
  redirectTo: string;
  /** Show a one-line hint that OAuth always re-uses the existing
   *  github.com / accounts.google.com session, so "create new account"
   *  expectations don't match reality. Default off; turn on for
   *  /signup since that's where the confusion lives. */
  showSwitchAccountHint?: boolean;
}) {
  const [loading, setLoading] = useState<Provider | null>(null);

  async function go(provider: Provider) {
    setLoading(provider);
    await signIn(provider, { redirectTo });
  }

  return (
    <div>
      <div className={styles.providers}>
        <button
          type="button"
          className={`${styles.provider} ${styles.providerGithub}`}
          onClick={() => go("github")}
          disabled={loading !== null}
          aria-label="Continue with GitHub"
        >
          <GithubIcon />
          <span>{loading === "github" ? "Redirecting…" : "Continue with GitHub"}</span>
        </button>

        <button
          type="button"
          className={`${styles.provider} ${styles.providerGoogle}`}
          onClick={() => go("google")}
          disabled={loading !== null}
          aria-label="Continue with Google"
        >
          <GoogleIcon />
          <span>{loading === "google" ? "Redirecting…" : "Continue with Google"}</span>
        </button>
      </div>

      {showSwitchAccountHint && (
        <p className={styles.providerHint}>
          GitHub or Google sign-in always re-uses the account you&apos;re
          already logged into at github.com / accounts.google.com — there&apos;s
          one ChakraMCP account per OAuth identity. To switch identities, sign
          out of the provider in another tab first, or use email + password
          below for a separate account.
        </p>
      )}
    </div>
  );
}

/* ─── Inline brand icons ─── */

function GithubIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
      <path
        fill="currentColor"
        d="M12 0a12 12 0 0 0-3.79 23.39c.6.11.82-.26.82-.58v-2.04c-3.34.73-4.04-1.61-4.04-1.61-.55-1.39-1.34-1.76-1.34-1.76-1.09-.75.08-.74.08-.74 1.21.09 1.85 1.24 1.85 1.24 1.07 1.84 2.81 1.31 3.49 1 .11-.78.42-1.31.76-1.61-2.67-.3-5.47-1.34-5.47-5.95 0-1.32.47-2.39 1.24-3.23-.13-.31-.54-1.54.11-3.21 0 0 1.01-.32 3.3 1.23a11.41 11.41 0 0 1 6 0c2.29-1.55 3.3-1.23 3.3-1.23.65 1.67.24 2.9.12 3.21.77.84 1.23 1.91 1.23 3.23 0 4.62-2.81 5.64-5.49 5.94.43.37.81 1.1.81 2.22v3.29c0 .32.22.7.83.58A12 12 0 0 0 12 0z"
      />
    </svg>
  );
}

function GoogleIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
      <path
        fill="#4285F4"
        d="M21.35 11.1H12v2.92h5.36c-.23 1.5-1.6 4.4-5.36 4.4-3.23 0-5.86-2.67-5.86-5.97s2.63-5.97 5.86-5.97c1.84 0 3.07.78 3.78 1.45l2.58-2.49C16.85 3.93 14.65 3 12 3 6.98 3 2.92 7.06 2.92 12.08S6.98 21.16 12 21.16c6.92 0 9.51-4.84 9.51-9.32 0-.62-.07-1.12-.16-1.74z"
      />
    </svg>
  );
}
