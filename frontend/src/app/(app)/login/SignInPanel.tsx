"use client";

import Link from "next/link";
import { useRef, useState } from "react";
import { signIn } from "next-auth/react";
import ReCAPTCHA from "react-google-recaptcha";
import styles from "./login.module.css";
import { OAuthProviders } from "./OAuthProviders";

type Provider = "password";

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
  // Masking toggle for the password input only (Show/Hide). The
  // email+password form is always rendered now — see the captcha
  // placement note in the JSX below.
  const [revealPassword, setRevealPassword] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  // If the captcha is enabled, the user must complete it before either
  // sign-in button works. We hold the v2 token here.
  const [captchaToken, setCaptchaToken] = useState<string | null>(null);

  const captchaReady = !captchaEnabled || !!captchaToken;
  const showCaptchaWidget = captchaEnabled && !!captchaSiteKey;
  const captchaMisconfigured = captchaEnabled && !captchaSiteKey;

  async function handlePasswordSignIn() {
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

    setLoadingProvider("password");
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
  }

  return (
    <div className={styles.panel}>
      <OAuthProviders redirectTo={redirectTo} />

      <div className={styles.divider}>
        <span>or</span>
      </div>

      <form
        className={styles.passwordForm}
        onSubmit={(e) => {
          e.preventDefault();
          handlePasswordSignIn();
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
          <div className={styles.passwordWrap}>
            <input
              type={revealPassword ? "text" : "password"}
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••"
              autoComplete="current-password"
            />
            <button
              type="button"
              className={styles.revealBtn}
              onClick={() => setRevealPassword((s) => !s)}
              aria-label={revealPassword ? "Hide password" : "Show password"}
              aria-pressed={revealPassword}
            >
              {revealPassword ? "Hide" : "Show"}
            </button>
          </div>
        </label>

        {/* The captcha gates ONLY the email/password sign-in — that's
            the endpoint whose token we actually verify server-side.
            GitHub/Google aren't gated (their auth happens at the
            provider). It sits right above the Sign in button it
            protects, rather than under the OAuth buttons where it
            misleadingly looked like it gated those too. */}
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

        <button
          type="submit"
          className={styles.submitBtn}
          disabled={!captchaReady || loadingProvider !== null}
        >
          {loadingProvider === "password" ? "Signing in…" : "Sign in"}
        </button>
      </form>

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
