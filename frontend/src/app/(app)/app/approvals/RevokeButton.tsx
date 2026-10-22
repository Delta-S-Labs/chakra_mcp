"use client";

import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { revokeAction } from "./actions";
import styles from "./approvals.module.css";

/** Revoke a standing consent; later requests ask again. */
export function RevokeButton({ consentId }: { consentId: string }) {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();
  return (
    <div>
      <button
        type="button"
        className={styles.btnSecondary}
        disabled={pending}
        onClick={() => {
          if (!confirm("Revoke this consent? Later requests will ask you again.")) return;
          setError(null);
          startTransition(async () => {
            const result = await revokeAction(consentId);
            if (result.ok) router.refresh();
            else setError(result.error);
          });
        }}
      >
        {pending ? "Revoking…" : "Revoke"}
      </button>
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
