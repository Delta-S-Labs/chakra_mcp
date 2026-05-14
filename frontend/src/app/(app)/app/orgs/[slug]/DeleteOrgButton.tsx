"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Modal } from "@/components/Modal";
import { deleteOrg } from "@/lib/api";
import styles from "./agent-actions.module.css";

/**
 * Owner-only "Delete org" button.
 *
 * Rendered in the org-detail page footer. On confirm, DELETE
 * /v1/orgs/{slug} — the backend handles re-parenting every agent into
 * the owner's personal account, removing all memberships, then dropping
 * the account row. Redirects to /app/orgs on success.
 */
export function DeleteOrgButton({
  token,
  slug,
  displayName,
  agentCount,
}: {
  token: string;
  slug: string;
  displayName: string;
  agentCount: number;
}) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();

  function handleConfirm() {
    setError(null);
    startTransition(async () => {
      try {
        await deleteOrg(token, slug);
        // Send the user back to the org list so they can see what's
        // left. router.refresh() isn't enough — the current page no
        // longer exists.
        router.push("/app/orgs");
      } catch (err) {
        setError(err instanceof Error ? err.message : "Delete failed.");
      }
    });
  }

  const agentNoun = agentCount === 1 ? "agent" : "agents";

  return (
    <>
      <section className={styles.dangerZone}>
        <h2 className={styles.dangerTitle}>Danger zone</h2>
        <div className={styles.dangerRow}>
          <div>
            <div className={styles.dangerLabel}>Delete organization</div>
            <p className={styles.dangerBody}>
              Moves the org&apos;s {agentCount} {agentNoun} to your personal
              account, removes every membership, then deletes the org. Cannot
              be undone.
            </p>
          </div>
          <button
            type="button"
            className={styles.dangerBtn}
            onClick={() => {
              setError(null);
              setOpen(true);
            }}
          >
            Delete org
          </button>
        </div>
      </section>

      <Modal
        open={open}
        title={`Delete ${displayName}?`}
        confirmLabel="Delete organization"
        tone="danger"
        busy={pending}
        onCancel={() => setOpen(false)}
        onConfirm={handleConfirm}
        body={
          <>
            <p>
              Delete <code>{slug}</code>? This moves {agentCount} {agentNoun} to
              your personal account, removes all org memberships, and deletes
              the org. This cannot be undone.
            </p>
            {error && <p className={styles.modalError}>{error}</p>}
          </>
        }
      />
    </>
  );
}
