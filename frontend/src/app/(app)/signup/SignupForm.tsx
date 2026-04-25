"use client";

import { useRef, useState } from "react";
import { signIn } from "next-auth/react";
import ReCAPTCHA from "react-google-recaptcha";
import { signupWithPassword } from "@/lib/api";
import styles from "../login/login.module.css";

export function SignupForm({
  captchaEnabled,
  captchaSiteKey,
  redirectTo,
}: {
  captchaEnabled: boolean;
  captchaSiteKey: string;
  redirectTo: string;
}) {
  const recaptchaRef = useRef<ReCAPTCHA>(null);
  const [email, setEmail] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [captchaToken, setCaptchaToken] = useState<string | null>(null);

  const showCaptchaWidget = captchaEnabled && !!captchaSiteKey;
  const captchaReady = !captchaEnabled || !!captchaToken;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
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
          setError(json.error || "Captcha verification failed.");
          recaptchaRef.current?.reset();
          setCaptchaToken(null);
          return;
        }
      } catch {
        setError("Captcha verification request failed.");
        return;
      }
    }

    setPending(true);
    try {
      // Create the account on the backend first.
      await signupWithPassword({ email: email.trim(), password, name: name.trim() });
      // Then immediately sign in via the Credentials provider so a session is set.
      // redirect: false returns a result object we can inspect; on success we navigate manually.
      const result = await signIn("password", {
        email: email.trim(),
        password,
        redirectTo,
        redirect: false,
      });
      if (result?.error) {
        setError("Account created, but sign-in failed. Try signing in.");
      } else {
        window.location.href = redirectTo;
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-up failed.");
    } finally {
      setPending(false);
    }
  }

  return (
    <form className={styles.passwordForm} onSubmit={handleSubmit}>
      <label className={styles.field}>
        <span className={styles.fieldLabel}>Name</span>
        <input
          type="text"
          required
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Your display name"
        />
      </label>

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
          minLength={8}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder="At least 8 characters"
          autoComplete="new-password"
        />
      </label>

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

      {error && <div className={styles.error}>{error}</div>}

      <button type="submit" className={styles.submitBtn} disabled={pending || !captchaReady}>
        {pending ? "Creating account…" : "Create account"}
      </button>
    </form>
  );
}
