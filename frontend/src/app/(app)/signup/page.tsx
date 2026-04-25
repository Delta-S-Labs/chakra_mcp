import Link from "next/link";
import { SignupForm } from "./SignupForm";
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
          <div className={styles.eyebrow}>Sign up</div>
          <h1 className={styles.title}>Create an account.</h1>
          <p className={styles.body}>
            Email and password works. So does GitHub and Google — there&apos;s a
            link to those at the bottom. Any of the three signs you in to the
            same account if the email matches.
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
