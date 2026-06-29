"use client";

import { useState, type ReactNode } from "react";
import { issueOAuthCode, ApiClientError } from "@/lib/api";
import styles from "./oauth.module.css";

export interface ConsentAgent {
  id: string;
  label: string;
  account: string;
}

type AgentScope = "all" | "own" | "selected";

function normalizeScope(v: string | null | undefined): AgentScope | null {
  return v === "all" || v === "own" || v === "selected" ? v : null;
}

export function ConsentForm({
  token,
  clientId,
  clientName,
  redirectUri,
  codeChallenge,
  codeChallengeMethod,
  state,
  scope,
  agents,
  requestedAgentScope,
  requestedAgentIds,
}: {
  token: string;
  clientId: string;
  clientName: string;
  redirectUri: string;
  codeChallenge: string;
  codeChallengeMethod: "S256";
  state: string;
  scope: string;
  agents: ConsentAgent[];
  requestedAgentScope?: string | null;
  requestedAgentIds?: string[];
}) {
  const requested = normalizeScope(requestedAgentScope);
  // Approved default: honour the client's request when present; otherwise
  // "all" — preserves behaviour for existing clients that send no request
  // (they'd otherwise be silently downgraded). The chooser is still here.
  const [agentScope, setAgentScope] = useState<AgentScope>(requested ?? "all");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => {
    const known = new Set(agents.map((a) => a.id));
    return new Set((requestedAgentIds ?? []).filter((id) => known.has(id)));
  });
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const needsPick = agentScope === "selected" && selectedIds.size === 0;

  function toggleAgent(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function handleApprove() {
    setError(null);
    setPending(true);
    try {
      const { code } = await issueOAuthCode(token, {
        client_id: clientId,
        redirect_uri: redirectUri,
        code_challenge: codeChallenge,
        code_challenge_method: codeChallengeMethod,
        scope,
        agent_scope: agentScope,
        ...(agentScope === "selected"
          ? { selected_agent_ids: Array.from(selectedIds) }
          : {}),
      });
      window.location.href = appendQuery(redirectUri, { code, state });
    } catch (err) {
      // A 401 here means the NextAuth session is still present but its
      // backend token has gone stale — bounce through /login (which mints
      // a fresh token) and return to this exact authorize URL so the OAuth
      // flow resumes.
      if (err instanceof ApiClientError && err.status === 401) {
        const back = window.location.pathname + window.location.search;
        window.location.href = `/login?from=${encodeURIComponent(back)}`;
        return;
      }
      setError(err instanceof Error ? err.message : "Couldn't issue code.");
      setPending(false);
    }
  }

  function handleDeny() {
    window.location.href = appendQuery(redirectUri, {
      error: "access_denied",
      error_description: "User denied the consent request.",
      state,
    });
  }

  return (
    <>
      <div className={styles.scopeSection}>
        <div className={styles.scopeLabel}>Agent access</div>
        {requested && (
          <div className={styles.reqNote}>
            <span aria-hidden="true">&#9432;</span>
            <span>
              {clientName} requested <strong>{requestedLabel(requested)}</strong>.
              You can widen or narrow this below.
            </span>
          </div>
        )}

        <ScopeOption
          value="own"
          current={agentScope}
          onSelect={setAgentScope}
          title={
            <>
              Only agents {clientName} creates
              {requested === "own" && <span className={styles.pill}>requested</span>}
            </>
          }
          desc={`${clientName} can create new agents and manage just those — it can never touch your other agents.`}
        />

        <ScopeOption
          value="selected"
          current={agentScope}
          onSelect={setAgentScope}
          title={
            <>
              Specific agents
              {requested === "selected" && (
                <span className={styles.pill}>requested</span>
              )}
            </>
          }
          desc="Pick exactly which existing agents it may manage."
        />
        {agentScope === "selected" && (
          <div className={styles.picker}>
            {agents.length === 0 ? (
              <div className={styles.pickerEmpty}>You have no agents yet.</div>
            ) : (
              agents.map((a) => (
                <label key={a.id} className={styles.agentRow}>
                  <input
                    type="checkbox"
                    checked={selectedIds.has(a.id)}
                    onChange={() => toggleAgent(a.id)}
                  />
                  {a.label} <span className={styles.meta}>&middot; {a.account}</span>
                </label>
              ))
            )}
          </div>
        )}

        <ScopeOption
          value="all"
          current={agentScope}
          onSelect={setAgentScope}
          title={
            <>
              Full access
              {requested === "all" && <span className={styles.pill}>requested</span>}
            </>
          }
          desc="Manage every agent in your accounts. Most permissive."
        />
      </div>

      <div className={styles.actions}>
        <button
          type="button"
          className={styles.approve}
          disabled={pending || needsPick}
          onClick={handleApprove}
        >
          {pending ? "Approving…" : "Approve"}
        </button>
        <button
          type="button"
          className={styles.deny}
          disabled={pending}
          onClick={handleDeny}
        >
          Deny
        </button>
      </div>
      {needsPick && (
        <div className={styles.hint}>
          Pick at least one agent, or choose a different access level.
        </div>
      )}
      {error && <div className={styles.error}>{error}</div>}
    </>
  );
}

function ScopeOption({
  value,
  current,
  onSelect,
  title,
  desc,
}: {
  value: AgentScope;
  current: AgentScope;
  onSelect: (v: AgentScope) => void;
  title: ReactNode;
  desc: string;
}) {
  const selected = current === value;
  return (
    <label className={`${styles.opt} ${selected ? styles.optSelected : ""}`.trim()}>
      <input
        type="radio"
        name="agent_scope"
        value={value}
        checked={selected}
        onChange={() => onSelect(value)}
      />
      <span style={{ display: "grid", gap: "0.1rem" }}>
        <span className={styles.optTitle}>{title}</span>
        <span className={styles.optDesc}>{desc}</span>
      </span>
    </label>
  );
}

function requestedLabel(s: AgentScope): string {
  if (s === "own") return "only the agents it creates";
  if (s === "selected") return "specific agents";
  return "full access";
}

function appendQuery(uri: string, params: Record<string, string>): string {
  const url = new URL(uri);
  for (const [k, v] of Object.entries(params)) {
    if (v) url.searchParams.set(k, v);
  }
  return url.toString();
}
