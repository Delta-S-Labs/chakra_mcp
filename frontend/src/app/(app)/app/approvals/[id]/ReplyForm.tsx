"use client";

import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { confirmReplyAction, denyAction } from "../actions";
import styles from "../approvals.module.css";

/**
 * Confirm a reply's draft as shown, or reject it with a note. If the agent
 * replaced the draft meanwhile, the relay says so and the page reloads it.
 */
export function ReplyForm({ id, draftSha256 }: { id: string; draftSha256: string }) {
  const router = useRouter();
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();

  function decide(kind: "confirm" | "reject") {
    setError(null);
    startTransition(async () => {
      const result =
        kind === "confirm" ? await confirmReplyAction(id, draftSha256) : await denyAction(id, note);
      if (!result.ok) setError(result.error);
      router.refresh();
    });
  }

  return (
    <div className={styles.card}>
      <label className={styles.field}>
        <span>
          Note if you reject <span className={styles.hint}>the requester sees it</span>
        </span>
        <input maxLength={500} value={note} onChange={(e) => setNote(e.target.value)} />
      </label>
      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.btnPrimary}
          disabled={pending}
          onClick={() => decide("confirm")}
        >
          {pending ? "Saving…" : "Confirm and send"}
        </button>
        <button
          type="button"
          className={styles.btnSecondary}
          disabled={pending}
          onClick={() => decide("reject")}
        >
          Reject
        </button>
      </div>
    </div>
  );
}
