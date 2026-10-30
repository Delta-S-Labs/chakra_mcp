"use client";

import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { replyOutput } from "@/lib/owner-reply";
import type { ReplyShape } from "@/lib/relay";
import { answerAction } from "../actions";
import styles from "../../approvals/approvals.module.css";

/**
 * The owner's answer: a reply in the capability's shape (text, or JSON when
 * it has an output schema other than `message_owner`'s), or a decline with
 * an optional reason. Sent straight to the caller.
 */
export function ReplyBox({ id, shape }: { id: string; shape: ReplyShape }) {
  const router = useRouter();
  const [reply, setReply] = useState("");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();

  function send(kind: "reply" | "decline") {
    setError(null);
    let answer: Parameters<typeof answerAction>[1];
    if (kind === "reply") {
      const built = replyOutput(shape, reply);
      if (!built.ok) {
        setError(built.error);
        return;
      }
      answer = { status: "succeeded", output: built.output };
    } else {
      answer = { status: "failed", error: reason.trim() || "The owner declined to answer." };
    }
    startTransition(async () => {
      const result = await answerAction(id, answer);
      if (!result.ok) setError(result.error);
      router.refresh();
    });
  }

  return (
    <div className={styles.card}>
      <label className={styles.field}>
        <span>
          {shape === "json" ? (
            <>
              Your reply, as JSON <span className={styles.hint}>this capability&apos;s output</span>
            </>
          ) : (
            "Your reply"
          )}
        </span>
        <textarea
          rows={6}
          maxLength={16000}
          value={reply}
          onChange={(e) => setReply(e.target.value)}
        />
      </label>
      <label className={styles.field}>
        <span>
          Reason if you decline <span className={styles.hint}>the caller sees it</span>
        </span>
        <input maxLength={500} value={reason} onChange={(e) => setReason(e.target.value)} />
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
          onClick={() => send("reply")}
        >
          {pending ? "Sending…" : "Send reply"}
        </button>
        <button
          type="button"
          className={styles.btnSecondary}
          disabled={pending}
          onClick={() => send("decline")}
        >
          Decline
        </button>
      </div>
    </div>
  );
}
