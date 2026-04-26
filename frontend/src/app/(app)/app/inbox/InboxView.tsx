"use client";

import { useState } from "react";
import {
  pullInbox,
  reportResult,
  type Agent,
  type Invocation,
} from "@/lib/relay";
import styles from "./inbox.module.css";

export function InboxView({
  token,
  myAgents,
}: {
  token: string | null;
  myAgents: Agent[];
}) {
  const [agentId, setAgentId] = useState(myAgents[0]?.id ?? "");
  const [items, setItems] = useState<Invocation[]>([]);
  const [pulling, setPulling] = useState(false);
  const [pullError, setPullError] = useState<string | null>(null);

  if (myAgents.length === 0) {
    return (
      <div className={styles.notice}>
        Register an agent first under <strong>Agents</strong>. Each agent
        gets its own inbox.
      </div>
    );
  }

  async function handlePull() {
    if (!token) {
      setPullError("Sign in again — no backend token.");
      return;
    }
    setPullError(null);
    setPulling(true);
    try {
      const claimed = await pullInbox(token, agentId);
      // Merge: drop existing rows that were re-claimed (shouldn't happen),
      // then prepend new ones.
      setItems((prev) => {
        const ids = new Set(claimed.map((c) => c.id));
        return [...claimed, ...prev.filter((p) => !ids.has(p.id))];
      });
    } catch (err) {
      setPullError(err instanceof Error ? err.message : "Couldn't pull inbox.");
    } finally {
      setPulling(false);
    }
  }

  function onResolved(updated: Invocation) {
    setItems((prev) => prev.map((p) => (p.id === updated.id ? updated : p)));
  }

  return (
    <>
      <section className={styles.controls}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Agent</span>
          <select value={agentId} onChange={(e) => setAgentId(e.target.value)}>
            {myAgents.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className={styles.create}
          disabled={pulling}
          onClick={handlePull}
        >
          {pulling ? "Pulling…" : "Pull inbox"}
        </button>
      </section>

      {pullError && <div className={styles.error}>{pullError}</div>}

      {items.length === 0 ? (
        <p className={styles.empty}>
          Nothing claimed yet. Hit <strong>Pull inbox</strong> to fetch the
          oldest pending invocations for this agent.
        </p>
      ) : (
        <ul className={styles.list}>
          {items.map((i) => (
            <Row key={i.id} token={token} item={i} onResolved={onResolved} />
          ))}
        </ul>
      )}
    </>
  );
}

function Row({
  token,
  item,
  onResolved,
}: {
  token: string | null;
  item: Invocation;
  onResolved: (i: Invocation) => void;
}) {
  const [output, setOutput] = useState("{}");
  const [errorText, setErrorText] = useState("");
  const [pending, setPending] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const isOpen = item.status === "in_progress";

  async function submit(status: "succeeded" | "failed") {
    if (!token) {
      setSubmitError("Sign in again — no backend token.");
      return;
    }
    let body;
    if (status === "succeeded") {
      let parsed: unknown;
      try {
        parsed = JSON.parse(output);
      } catch {
        setSubmitError("Output must be valid JSON.");
        return;
      }
      body = { status: "succeeded" as const, output: parsed };
    } else {
      body = {
        status: "failed" as const,
        error: errorText.trim() || "failed",
      };
    }
    setSubmitError(null);
    setPending(true);
    try {
      const updated = await reportResult(token, item.id, body);
      onResolved(updated);
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : "Couldn't post result.");
    } finally {
      setPending(false);
    }
  }

  return (
    <li className={styles.row}>
      <div className={styles.rowHead}>
        <div className={styles.rowName}>
          <code className={styles.capCode}>{item.capability_name}</code>
          <span className={styles.arrow}>←</span>
          <strong>{item.grantee_display_name ?? "deleted agent"}</strong>
          <StatusBadge status={item.status} />
        </div>
        <div className={styles.rowMeta}>
          <span>queued {new Date(item.created_at).toLocaleTimeString()}</span>
          {item.claimed_at && (
            <>
              <span>·</span>
              <span>claimed {new Date(item.claimed_at).toLocaleTimeString()}</span>
            </>
          )}
        </div>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Input</div>
        <pre className={styles.pre}>{JSON.stringify(item.input_preview, null, 2)}</pre>
      </div>

      {isOpen ? (
        <div className={styles.respond}>
          <div className={styles.respondTabs}>
            <div className={styles.field}>
              <span className={styles.fieldLabel}>Output JSON (for succeeded)</span>
              <textarea
                rows={3}
                value={output}
                onChange={(e) => setOutput(e.target.value)}
                placeholder='{"summary":"…"}'
              />
            </div>
            <div className={styles.field}>
              <span className={styles.fieldLabel}>Error (for failed)</span>
              <textarea
                rows={3}
                value={errorText}
                onChange={(e) => setErrorText(e.target.value)}
                placeholder="What went wrong?"
              />
            </div>
          </div>
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.create}
              disabled={pending}
              onClick={() => submit("succeeded")}
            >
              {pending ? "Sending…" : "Mark succeeded"}
            </button>
            <button
              type="button"
              className={styles.dangerBtn}
              disabled={pending}
              onClick={() => submit("failed")}
            >
              Mark failed
            </button>
          </div>
          {submitError && <div className={styles.error}>{submitError}</div>}
        </div>
      ) : (
        <>
          {item.output_preview != null && (
            <div className={styles.section}>
              <div className={styles.sectionTitle}>Output</div>
              <pre className={styles.pre}>
                {JSON.stringify(item.output_preview, null, 2)}
              </pre>
            </div>
          )}
          {item.error_message && (
            <div className={styles.section}>
              <div className={styles.sectionTitle}>Error</div>
              <p className={styles.errorText}>{item.error_message}</p>
            </div>
          )}
        </>
      )}
    </li>
  );
}

function StatusBadge({ status }: { status: Invocation["status"] }) {
  const cls =
    status === "succeeded"
      ? styles.badgeOk
      : status === "in_progress"
      ? styles.badgeWarn
      : status === "pending"
      ? styles.badgeNeutral
      : styles.badgeBad;
  return <span className={`${styles.badge} ${cls}`}>{status}</span>;
}
