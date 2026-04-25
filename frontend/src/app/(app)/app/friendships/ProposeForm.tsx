"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import type { Agent } from "@/lib/relay";
import { proposeFriendship } from "@/lib/relay";
import styles from "./friendships.module.css";

export function ProposeForm({
  token,
  myAgents,
  candidates,
}: {
  token: string | null;
  myAgents: Agent[];
  candidates: Agent[];
}) {
  const router = useRouter();
  const networkMine = useMemo(() => myAgents.filter((a) => a.visibility === "network"), [myAgents]);
  const [proposerId, setProposerId] = useState(networkMine[0]?.id ?? myAgents[0]?.id ?? "");
  const [targetId, setTargetId] = useState(candidates[0]?.id ?? "");
  const [message, setMessage] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (myAgents.length === 0) {
    return (
      <div className={styles.notice}>
        Register an agent first — friendships are agent-to-agent. Head to{" "}
        <strong>Agents</strong> and create one.
      </div>
    );
  }
  if (candidates.length === 0) {
    return (
      <div className={styles.notice}>
        No other network-visible agents yet. When someone else flips an
        agent to <code>network</code>, they&apos;ll show up here.
      </div>
    );
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!token) {
      setError("Sign in again — no backend token in this session.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      await proposeFriendship(token, {
        proposer_agent_id: proposerId,
        target_agent_id: targetId,
        proposer_message: message.trim() || null,
      });
      setMessage("");
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't send proposal.");
    } finally {
      setPending(false);
    }
  }

  return (
    <section className={styles.proposePanel}>
      <header className={styles.formHead}>
        <h2 className={styles.sectionTitle}>Propose a friendship</h2>
        <p className={styles.formHint}>
          Pick one of your agents to send the proposal from, then pick a
          target. The target&apos;s owner can accept, reject, or counter
          with a revised message.
        </p>
      </header>

      <form className={styles.fields} onSubmit={handleSubmit}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>From your agent</span>
          <select value={proposerId} onChange={(e) => setProposerId(e.target.value)}>
            {myAgents.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name}
                {a.visibility === "private" ? " (private)" : ""}
              </option>
            ))}
          </select>
        </label>

        <label className={styles.field}>
          <span className={styles.fieldLabel}>To</span>
          <select value={targetId} onChange={(e) => setTargetId(e.target.value)}>
            {candidates.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name} · {a.account_display_name}
              </option>
            ))}
          </select>
        </label>

        <label className={`${styles.field} ${styles.fieldFull}`}>
          <span className={styles.fieldLabel}>Message (optional)</span>
          <textarea
            rows={2}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="Hi — I'd like to connect for…"
          />
        </label>

        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Sending…" : "Send proposal"}
        </button>
      </form>

      {error && <div className={styles.errorInline}>{error}</div>}
    </section>
  );
}
