"use client";

import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { narrowingOptions, validityLabel } from "@/lib/consent";
import type { ConsentTerms } from "@/lib/relay";
import { approveAction, denyAction } from "../actions";
import styles from "../approvals.module.css";

/**
 * Approve (as asked, or narrowed) or deny with a note. Only shorter
 * windows and smaller limits are offered; the relay checks again.
 */
export function DecisionForm({
  id,
  terms,
  maxValidity,
}: {
  id: string;
  terms: ConsentTerms;
  maxValidity: string;
}) {
  const router = useRouter();
  const options = narrowingOptions(terms.valid_for, maxValidity);
  const [validFor, setValidFor] = useState(options[0]);
  const [maxRequests, setMaxRequests] = useState<string>(
    terms.max_requests ? String(terms.max_requests) : "",
  );
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();
  const standing = validFor !== "once";

  function decide(kind: "approve" | "deny") {
    setError(null);
    let limit: number | null = null;
    if (kind === "approve" && standing && maxRequests.trim() !== "") {
      limit = Number(maxRequests);
      if (!Number.isInteger(limit) || limit < 1) {
        setError("The request limit must be a whole number, 1 or more.");
        return;
      }
      if (terms.max_requests && limit > terms.max_requests) {
        setError(`They asked for at most ${terms.max_requests} requests.`);
        return;
      }
    }
    startTransition(async () => {
      const result =
        kind === "approve" ? await approveAction(id, validFor, limit) : await denyAction(id, note);
      if (!result.ok) {
        setError(result.error);
        return;
      }
      router.refresh();
    });
  }

  return (
    <div className={styles.card}>
      <fieldset className={styles.choices}>
        <legend>Approve for</legend>
        {options.map((option) => (
          <label key={option} className={styles.choice}>
            <input
              type="radio"
              name="valid_for"
              value={option}
              checked={validFor === option}
              onChange={() => setValidFor(option)}
            />
            {validityLabel(option)}
          </label>
        ))}
      </fieldset>
      {standing && (
        <label className={styles.field}>
          <span>
            At most this many requests{" "}
            <span className={styles.hint}>
              {terms.max_requests ? `up to ${terms.max_requests}` : "blank for no limit"}
            </span>
          </span>
          <input
            inputMode="numeric"
            value={maxRequests}
            onChange={(e) => setMaxRequests(e.target.value)}
          />
        </label>
      )}
      <label className={styles.field}>
        <span>
          Note if you deny <span className={styles.hint}>the requester sees it</span>
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
          onClick={() => decide("approve")}
        >
          {pending ? "Saving…" : "Approve"}
        </button>
        <button
          type="button"
          className={styles.btnSecondary}
          disabled={pending}
          onClick={() => decide("deny")}
        >
          Deny
        </button>
      </div>
    </div>
  );
}
