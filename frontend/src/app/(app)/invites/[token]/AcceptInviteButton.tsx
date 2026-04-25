"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { acceptInvite } from "@/lib/api";
import styles from "../../login/login.module.css";

export function AcceptInviteButton({
  inviteToken,
  sessionToken,
}: {
  inviteToken: string;
  sessionToken: string;
}) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleAccept() {
    setError(null);
    setPending(true);
    try {
      const org = await acceptInvite(sessionToken, inviteToken);
      router.push(`/app/orgs/${org.slug}`);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to accept invite.");
      setPending(false);
    }
  }

  return (
    <div className={styles.passwordForm}>
      <button
        type="button"
        className={styles.submitBtn}
        onClick={handleAccept}
        disabled={pending}
      >
        {pending ? "Joining…" : "Accept and join"}
      </button>
      {error && <div className={styles.error}>{error}</div>}
    </div>
  );
}
