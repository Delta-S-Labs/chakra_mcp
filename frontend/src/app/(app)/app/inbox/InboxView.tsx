"use client";

import { useState, useTransition } from "react";
import {
  listInvocations,
  pullInbox,
  reportResult,
  type Agent,
  type Invocation,
} from "@/lib/relay";
import styles from "./inbox.module.css";

export function InboxView({
  token,
  myAgents,
  initialItems = [],
}: {
  token: string | null;
  myAgents: Agent[];
  initialItems?: Invocation[];
}) {
  const [agentId, setAgentId] = useState(myAgents[0]?.id ?? "");
  const [direction, setDirection] = useState<"inbound" | "outbound">("inbound");
  const [items, setItems] = useState<Invocation[]>(initialItems);
  const [pulling, setPulling] = useState(false);
  const [pullError, setPullError] = useState<string | null>(null);
  const [refreshing, startRefresh] = useTransition();

  if (myAgents.length === 0) {
    return (
      <div className={styles.notice}>
        Register an agent first under <strong>Agents</strong>. Each agent
        gets its own inbox.
      </div>
    );
  }

  // List the full queue for `id` in `dir` (every status), no claiming.
  // inbound = calls served BY this agent; outbound = calls it issued.
  // Used on agent/tab switch and after a pull so already-claimed
  // in-flight rows and finished rows stay visible.
  function refresh(id: string, dir: "inbound" | "outbound") {
    if (!token || !id) return;
    setPullError(null);
    startRefresh(async () => {
      try {
        const rows = await listInvocations(token, { direction: dir, agent_id: id });
        setItems(rows);
      } catch (err) {
        setPullError(
          err instanceof Error ? err.message : "Couldn't load invocations.",
        );
      }
    });
  }

  function onPickAgent(next: string) {
    if (next === agentId) return;
    setAgentId(next);
    setItems([]);
    refresh(next, direction);
  }

  function onPickDirection(next: "inbound" | "outbound") {
    if (next === direction) return;
    setDirection(next);
    setItems([]);
    refresh(agentId, next);
  }

  async function handlePull() {
    if (!token) {
      setPullError("Sign in again - no backend token.");
      return;
    }
    setPullError(null);
    setPulling(true);
    try {
      await pullInbox(token, agentId);
      // Re-list so the just-claimed rows AND any pre-existing in-flight /
      // finished rows all show with current status.
      refresh(agentId, direction);
    } catch (err) {
      setPullError(err instanceof Error ? err.message : "Couldn't pull inbox.");
    } finally {
      setPulling(false);
    }
  }

  function onResolved(updated: Invocation) {
    setItems((prev) => prev.map((p) => (p.id === updated.id ? updated : p)));
  }

  const isInbound = direction === "inbound";
  const pendingCount = items.filter((i) => i.status === "pending").length;

  return (
    <>
      <div className={styles.tabs} role="tablist" aria-label="Direction">
        <button
          type="button"
          role="tab"
          aria-selected={isInbound}
          className={`${styles.tab} ${isInbound ? styles.tabActive : ""}`}
          onClick={() => onPickDirection("inbound")}
        >
          Inbound — work for this agent
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={!isInbound}
          className={`${styles.tab} ${!isInbound ? styles.tabActive : ""}`}
          onClick={() => onPickDirection("outbound")}
        >
          Outbound — calls it made
        </button>
      </div>

      <section className={styles.controls}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Agent</span>
          <select
            value={agentId}
            onChange={(e) => onPickAgent(e.target.value)}
          >
            {myAgents.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name}
              </option>
            ))}
          </select>
        </label>
        {isInbound && (
          <button
            type="button"
            className={styles.create}
            disabled={pulling || refreshing || pendingCount === 0}
            onClick={handlePull}
            title={
              pendingCount === 0
                ? "No unclaimed pending calls to pull"
                : undefined
            }
          >
            {pulling
              ? "Pulling…"
              : pendingCount > 0
              ? `Pull inbox (${pendingCount})`
              : "Pull inbox"}
          </button>
        )}
        <button
          type="button"
          className={styles.refresh}
          disabled={refreshing || pulling}
          onClick={() => refresh(agentId, direction)}
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
      </section>

      {pullError && <div className={styles.error}>{pullError}</div>}

      {items.length === 0 ? (
        <p className={styles.empty}>
          {isInbound ? (
            <>
              No invocations for this agent yet. When a peer invokes one of
              its capabilities, the call shows up here — pending, in-flight,
              or finished.
            </>
          ) : (
            <>
              This agent hasn&apos;t made any calls yet. Capabilities it
              invokes on other agents show up here — including ones still
              waiting on a reply.
            </>
          )}
        </p>
      ) : (
        <ul className={styles.list}>
          {items.map((i) => (
            <Row
              key={i.id}
              token={token}
              item={i}
              direction={direction}
              onResolved={onResolved}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function Row({
  token,
  item,
  direction,
  onResolved,
}: {
  token: string | null;
  item: Invocation;
  direction: "inbound" | "outbound";
  onResolved: (i: Invocation) => void;
}) {
  const [output, setOutput] = useState("{}");
  const [errorText, setErrorText] = useState("");
  const [pending, setPending] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  // The respond form only applies INBOUND (work you serve). For outbound
  // calls you're the caller — you can't (and shouldn't) post a result;
  // just watch the status until the other side answers.
  const isOpen = direction === "inbound" && item.status === "in_progress";

  async function submit(status: "succeeded" | "failed") {
    if (!token) {
      setSubmitError("Sign in again - no backend token.");
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
          {direction === "inbound" ? (
            <>
              <span className={styles.arrow}>←</span>
              <strong>{item.grantee_display_name ?? "deleted agent"}</strong>
            </>
          ) : (
            <>
              <span className={styles.arrow}>→</span>
              <strong>{item.granter_display_name ?? "deleted agent"}</strong>
            </>
          )}
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
