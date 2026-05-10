import Link from "next/link";
import { SignupForm } from "./SignupForm";
import { AlreadySignedIn } from "../login/AlreadySignedIn";
import styles from "../login/login.module.css";

export default async function SignupPage({
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
          <AlreadySignedIn />

          <div className={styles.eyebrow}>Sign up</div>
          <h1 className={styles.title}>Create an account.</h1>
          <p className={styles.body}>
            GitHub and Google work below — they sign you in if you already have
            an account, or create one if you don&apos;t. Email and password is
            the alternative if you want a separate account from your OAuth
            identity.
          </p>

          <SignupForm
            captchaEnabled={captchaEnabled}
            captchaSiteKey={captchaSiteKey}
            redirectTo={from || "/app"}
          />

          <p className={styles.foot}>
            Already have an account? <Link href="/login">Sign in</Link>. By
            creating an account you agree to the <Link href="/terms">terms</Link>.
          </p>
        </div>
      </div>
    </main>
  );
}
