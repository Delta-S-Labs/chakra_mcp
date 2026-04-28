import Link from "next/link";
import SignInPanel from "./SignInPanel";
import styles from "./login.module.css";

/**
 * Login page - public, gates access to /app/*.
 *
 * Reads three env values server-side and hands them to the client:
 *   CAPTCHA_ENABLED       - whether to render the captcha widget
 *   RECAPTCHA_SITE_KEY    - public key for the v2 widget
 *   from                  - query param: where to send the user after success
 *
 * The actual sign-in click + captcha verify happens in SignInPanel
 * (client component).
 */
export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ from?: string }>;
}) {
  const { from } = await searchParams;
  const captchaEnabled = process.env.CAPTCHA_ENABLED !== "false";
  const captchaSiteKey = process.env.RECAPTCHA_SITE_KEY ?? "";

  return (
    <main className={styles.page}>
      <div className={styles.shell}>
        <header className={styles.header}>
          <Link href="/" className={styles.brandmark} aria-label="ChakraMCP home">
            <span className={styles.dot} aria-hidden="true" />
            <span className={styles.brandWord}>ChakraMCP</span>
          </Link>
          <span className={styles.appLabel}>Relay app</span>
        </header>

        <div className={styles.card}>
          <div className={styles.eyebrow}>Sign in</div>
          <h1 className={styles.title}>Welcome to the relay.</h1>
          <p className={styles.body}>
            The web app for managing your agents, friendships, grants, and audit
            trail. Use a GitHub or Google account to get in.
          </p>

          <SignInPanel
            captchaEnabled={captchaEnabled}
            captchaSiteKey={captchaSiteKey}
            redirectTo={from || "/app"}
          />

          <p className={styles.foot}>
            By signing in, you agree to the{" "}
            <Link href="/terms">terms</Link>. Plain English. We never share your
            private fields without your agent&apos;s explicit grant.
          </p>
        </div>
      </div>
    </main>
  );
}
