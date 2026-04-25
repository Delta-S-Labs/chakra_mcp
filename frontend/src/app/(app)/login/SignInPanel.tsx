"use client";

import Link from "next/link";
import { useRef, useState } from "react";
import { signIn } from "next-auth/react";
import ReCAPTCHA from "react-google-recaptcha";
import styles from "./login.module.css";

type Provider = "github" | "google" | "password";

export default function SignInPanel({
  captchaEnabled,
  captchaSiteKey,
  redirectTo,
}: {
  captchaEnabled: boolean;
  captchaSiteKey: string;
  redirectTo: string;
}) {
  const recaptchaRef = useRef<ReCAPTCHA>(null);
  const [loadingProvider, setLoadingProvider] = useState<Provider | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showPassword, setShowPassword] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  // If the captcha is enabled, the user must complete it before either
  // sign-in button works. We hold the v2 token here.
  const [captchaToken, setCaptchaToken] = useState<string | null>(null);

  const captchaReady = !captchaEnabled || !!captchaToken;
  const showCaptchaWidget = captchaEnabled && !!captchaSiteKey;
  const captchaMisconfigured = captchaEnabled && !captchaSiteKey;

  async function handleSignIn(provider: Provider) {
    setError(null);

    if (captchaEnabled) {
      if (!captchaToken) {
        setError("Solve the captcha first.");
        return;
      }
      try {
        const res = await fetch("/api/captcha/verify", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ token: captchaToken }),
        });
        const json = await res.json();
        if (!res.ok || !json.ok) {
          setError(json.error || "Captcha verification failed. Try again.");
          recaptchaRef.current?.reset();
          setCaptchaToken(null);
          return;
        }
      } catch {
        setError("Captcha verification request failed. Try again.");
        return;
      }
    }

    setLoadingProvider(provider);
    if (provider === "password") {
      const result = await signIn("password", {
        email: email.trim(),
        password,
        redirectTo,
        redirect: false,
      });
      setLoadingProvider(null);
      if (result?.error) {
        setError("Wrong email or password.");
      } else if (result?.ok) {
        window.location.href = redirectTo;
      }
    } else {
      await signIn(provider, { redirectTo });
    }
  }

  return (
    <div className={styles.panel}>
      <div className={styles.providers}>
        <button
          type="button"
          className={`${styles.provider} ${styles.providerGithub}`}
          onClick={() => handleSignIn("github")}
          disabled={!captchaReady || loadingProvider !== null}
          aria-label="Continue with GitHub"
        >
          <GithubIcon />
          <span>{loadingProvider === "github" ? "Redirecting…" : "Continue with GitHub"}</span>
        </button>

        <button
          type="button"
          className={`${styles.provider} ${styles.providerGoogle}`}
          onClick={() => handleSignIn("google")}
          disabled={!captchaReady || loadingProvider !== null}
          aria-label="Continue with Google"
        >
          <GoogleIcon />
          <span>{loadingProvider === "google" ? "Redirecting…" : "Continue with Google"}</span>
        </button>
      </div>

      {showCaptchaWidget && (
        <div className={styles.captcha}>
          <ReCAPTCHA
            ref={recaptchaRef}
            sitekey={captchaSiteKey}
            onChange={(token) => {
              setCaptchaToken(token);
              setError(null);
            }}
            onExpired={() => setCaptchaToken(null)}
            onErrored={() => setError("Captcha widget failed to load.")}
          />
        </div>
      )}

      <div className={styles.divider}>
        <span>or</span>
      </div>

      {!showPassword ? (
        <button
          type="button"
          className={styles.secondaryBtn}
          onClick={() => setShowPassword(true)}
        >
          Sign in with email + password
        </button>
      ) : (
        <form
          className={styles.passwordForm}
          onSubmit={(e) => {
            e.preventDefault();
            handleSignIn("password");
          }}
        >
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Email</span>
            <input
              type="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
              autoComplete="email"
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Password</span>
            <input
              type="password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••"
              autoComplete="current-password"
            />
          </label>
          <button
            type="submit"
            className={styles.submitBtn}
            disabled={!captchaReady || loadingProvider !== null}
          >
            {loadingProvider === "password" ? "Signing in…" : "Sign in"}
          </button>
        </form>
      )}

      <p className={styles.signupHint}>
        New here? <Link href="/signup">Create an account</Link>.
      </p>

      {captchaMisconfigured && (
        <div className={styles.notice}>
          Captcha is enabled but no <code>RECAPTCHA_SITE_KEY</code> is set in
          <code> .env.local</code>. Add the keys, or set
          <code> CAPTCHA_ENABLED=false</code> for private deployments.
        </div>
      )}

      {error && <div className={styles.error}>{error}</div>}

      {!captchaEnabled && (
        <div className={styles.captchaOff}>Captcha disabled by env (private network mode).</div>
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
