"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Modal } from "@/components/Modal";
import { moveAgentToPersonal, type MoveToPersonalResponse } from "@/lib/api";
import styles from "./agent-actions.module.css";

/**
 * Inline "Move to personal account" button rendered on each agent row
 * inside an org-detail page. Re-parenting is destructive enough that
 * we put it behind a confirmation modal; the modal copy spells out the
 * collision-suffix behavior because the new slug may not match what
 * the user expected.
 */
export function MoveAgentButton({
  token,
  agentId,
  agentDisplayName,
  agentSlug,
  fromAccountSlug,
}: {
  token: string;
  agentId: string;
  agentDisplayName: string;
  agentSlug: string;
  fromAccountSlug: string;
}) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<MoveToPersonalResponse | null>(null);
  const [pending, startTransition] = useTransition();

  function handleConfirm() {
    setError(null);
    startTransition(async () => {
      try {
        const r = await moveAgentToPersonal(token, agentId);
        setResult(r);
        setOpen(false);
        // Refresh the server-rendered list so the agent disappears
        // from this org.
        router.refresh();
      } catch (err) {
        setError(err instanceof Error ? err.message : "Move failed.");
      }
    });
  }

  return (
    <>
      <button
        type="button"
        className={styles.actionBtn}
        onClick={() => {
          setError(null);
          setResult(null);
          setOpen(true);
        }}
      >
        Move to personal
      </button>

      <Modal
        open={open}
        title={`Move ${agentDisplayName} to your personal account?`}
        confirmLabel="Move agent"
        tone="neutral"
        busy={pending}
        onCancel={() => setOpen(false)}
        onConfirm={handleConfirm}
        body={
          <>
            <p>
              The agent <code>{fromAccountSlug}/{agentSlug}</code> will be
              re-parented to your personal account. Friendships, grants, and
              invocation history all survive — only the parent account changes.
            </p>
            <p>
              If your personal account already has an agent with the slug
              <code> {agentSlug}</code>, a numeric suffix is appended (e.g.{" "}
              <code>{agentSlug}-2</code>).
            </p>
            {error && <p className={styles.modalError}>{error}</p>}
          </>
        }
      />

      {result && (
        <p className={styles.movedNote}>
          Moved to{" "}
          <code>
            {result.new_account_slug}/{result.new_agent_slug}
          </code>
          {result.new_agent_slug !== agentSlug && " (slug changed)"}.
        </p>
      )}
    </>
  );
}
