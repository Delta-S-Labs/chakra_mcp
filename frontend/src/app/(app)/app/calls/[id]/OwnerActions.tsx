"use client";

import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { deleteContentAction, endCallAction, type CallActionResult } from "../actions";
import styles from "../calls.module.css";

/** An owner's actions on a call: end it, or delete its content now. */
export function OwnerActions({
  callId,
  canEnd,
  canDelete,
}: {
  callId: string;
  canEnd: boolean;
  canDelete: boolean;
}) {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();
  if (!canEnd && !canDelete) return null;

  const run = (question: string, action: () => Promise<CallActionResult>) => {
    if (!confirm(question)) return;
    setError(null);
    startTransition(async () => {
      const result = await action();
      if (result.ok) router.refresh();
      else setError(result.error);
    });
  };

  return (
    <div className={styles.actions}>
      {canEnd && (
        <button
          type="button"
          className={styles.btnPrimary}
          disabled={pending}
          onClick={() =>
            run("End this call? Both agents will be told.", () => endCallAction(callId))
          }
        >
          End call
        </button>
      )}
      {canDelete && (
        <button
          type="button"
          className={styles.btnSecondary}
          disabled={pending}
          onClick={() =>
            run(
              "Delete this call's content now, for both sides? Messages and the memory go; " +
                "a record that the call happened stays. This can't be undone.",
              () => deleteContentAction(callId),
            )
          }
        >
          Delete content now
        </button>
      )}
      {error && (
        <p className={styles.actionError} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
