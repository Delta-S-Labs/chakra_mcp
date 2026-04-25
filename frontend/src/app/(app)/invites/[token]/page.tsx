import Link from "next/link";
import { auth } from "@/auth";
import { previewInvite, type InvitePreview } from "@/lib/api";
import { AcceptInviteButton } from "./AcceptInviteButton";
import styles from "../../login/login.module.css";

export default async function InviteAcceptPage({
  params,
}: {
  params: Promise<{ token: string }>;
}) {
  const { token } = await params;
  const session = await auth();
  const backendToken = session?.backendToken;

  let preview: InvitePreview | null = null;
  let previewError: string | null = null;
  try {
    preview = await previewInvite(token);
  } catch (err) {
    previewError = err instanceof Error ? err.message : "Invite link is invalid.";
  }

  return (
    <main className={styles.page}>
      <div className={styles.shell}>
        <header className={styles.header}>
          <Link href="/" className={styles.brandmark} aria-label="ChakraMCP home">
            <span className={styles.dot} aria-hidden="true" />
            <span className={styles.brandWord}>ChakraMCP</span>
          </Link>
          <span className={styles.appLabel}>Invite</span>
        </header>

        <div className={styles.card}>
          <div className={styles.eyebrow}>Invitation</div>
          {previewError ? (
            <>
              <h1 className={styles.title}>Invite link not usable.</h1>
              <p className={styles.body}>{previewError}</p>
              <p className={styles.foot}>
                If the link is old, ask the inviter for a fresh one.
              </p>
            </>
          ) : preview ? (
            <>
              <h1 className={styles.title}>Join {preview.org_display_name}.</h1>
              <p className={styles.body}>
                You&apos;re invited to <strong>{preview.org_display_name}</strong>{" "}
                (<code>{preview.org_slug}</code>) as a <strong>{preview.role}</strong>.
                The invite was sent to <code>{preview.email}</code>.
              </p>

              {!session?.user ? (
                <p className={styles.foot}>
                  <Link href={`/login?from=/invites/${token}`}>Sign in</Link> with
                  the email <code>{preview.email}</code> to accept. (Or{" "}
                  <Link href={`/signup?from=/invites/${token}`}>create an account</Link>.)
                </p>
              ) : session.user.email?.toLowerCase() !== preview.email.toLowerCase() ? (
                <p className={styles.foot}>
                  You&apos;re signed in as <code>{session.user.email}</code>, but the invite is for{" "}
                  <code>{preview.email}</code>.{" "}
                  <Link href="/api/auth/signout">Sign out</Link> and sign in as the right
                  account.
                </p>
              ) : backendToken ? (
                <AcceptInviteButton inviteToken={token} sessionToken={backendToken} />
              ) : (
                <p className={styles.foot}>
                  Your session is missing the backend token — sign in again.
                </p>
              )}
            </>
          ) : (
            <p className={styles.body}>Loading…</p>
          )}
        </div>
      </div>
    </main>
  );
}
