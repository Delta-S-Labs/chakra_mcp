"use client";

import { useState } from "react";
import Link from "next/link";
import {
  approveDeviceSession,
  denyDeviceSession,
  ApiClientError,
} from "@/lib/api";
import styles from "./pair.module.css";

export interface PairableAgent {
  id: string;
  slug: string;
  display_name: string;
  account_slug: string;
}

/**
 * Consent UI for an in-flight device-flow pairing session.
 *
 * Two modes:
 *   - "create": POST /oauth/device-approve with slug/display_name →
 *     backend creates a fresh agent record and binds it to the
 *     device_code.
 *   - "existing": POST /oauth/device-approve with existing_agent_id →
 *     backend binds the device_code to an agent the user already owns
 *     (re-pairing / authenticating a known agent on a new machine).
 *
 * Either way the agent's next /oauth/token poll returns the access token.
 *
 * On Deny: POST /oauth/device-deny → backend marks denied_at. Agent
 * sees access_denied on next poll and stops.
 */
export function ConsentForm({
  token,
  userCode,
  slugHint,
  displayNameHint,
  descriptionHint,
  visibilityHint,
  existingAgents = [],
}: {
  token: string;
  userCode: string;
  slugHint: string;
  displayNameHint: string;
  descriptionHint: string;
  visibilityHint: string;
  existingAgents?: PairableAgent[];
}) {
  const [mode, setMode] = useState<"create" | "existing">("create");
  const [existingAgentId, setExistingAgentId] = useState<string>(
    existingAgents[0]?.id ?? "",
  );
  const [slug, setSlug] = useState(slugHint);
  const [displayName, setDisplayName] = useState(
    displayNameHint || titleCase(slugHint),
  );
  const [description, setDescription] = useState(descriptionHint);
  // Pre-fill from the client's hint; anything but an explicit "network"
  // (empty, unknown) falls back to the safe default. User can still change it.
  const [visibility, setVisibility] = useState<"private" | "network">(
    visibilityHint === "network" ? "network" : "private",
  );

  const [status, setStatus] = useState<
    "idle" | "approving" | "denying" | "approved" | "denied"
  >("idle");
  const [error, setError] = useState<string | null>(null);
  const [approvedAgent, setApprovedAgent] = useState<{
    slug: string;
    account_slug: string;
  } | null>(null);

  async function handleApprove(e: React.FormEvent) {
    e.preventDefault();
    setError(null);

    const pairingExisting = mode === "existing";
    if (pairingExisting) {
      if (!existingAgentId) {
        setError("Pick an agent to pair.");
        return;
      }
    } else {
      if (!isValidSlug(slug)) {
        setError(
          "Slug must be 3-32 chars: lowercase letters, digits, hyphens. No leading/trailing hyphen.",
        );
        return;
      }
      if (!displayName.trim()) {
        setError("Display name is required.");
        return;
      }
    }

    setStatus("approving");
    try {
      const res = await approveDeviceSession(
        token,
        pairingExisting
          ? { user_code: userCode, existing_agent_id: existingAgentId }
          : {
              user_code: userCode,
              agent_slug: slug.trim(),
              agent_display_name: displayName.trim(),
              agent_description: description.trim() || undefined,
              agent_visibility: visibility,
            },
      );
      setApprovedAgent({
        slug: res.agent_slug,
        account_slug: res.account_slug,
      });
      setStatus("approved");
    } catch (err) {
      setStatus("idle");
      if (err instanceof ApiClientError) {
        setError(err.message || `Approve failed (${err.status}).`);
      } else {
        setError(err instanceof Error ? err.message : "Approve failed.");
      }
    }
  }

  async function handleDeny() {
    setError(null);
    setStatus("denying");
    try {
      await denyDeviceSession(token, userCode);
      setStatus("denied");
    } catch (err) {
      setStatus("idle");
      setError(err instanceof Error ? err.message : "Deny failed.");
    }
  }

  if (status === "approved" && approvedAgent) {
    return (
      <div className={styles.success}>
        <p>
          <strong>Approved.</strong> The agent should pick up its credential
          on the next poll and start running. It&apos;s registered as{" "}
          <code>
            {approvedAgent.account_slug}/{approvedAgent.slug}
          </code>
          . You can manage it from <Link href="/app/agents">your agents</Link>.
        </p>
      </div>
    );
  }

  if (status === "denied") {
    return (
      <div className={styles.success}>
        <p>
          <strong>Denied.</strong> The agent will see <code>access_denied</code>
          {" "}on its next poll and stop. You can close this tab.
        </p>
      </div>
    );
  }

  const busy = status === "approving" || status === "denying";

  return (
    <form onSubmit={handleApprove}>
      {existingAgents.length > 0 && (
        <div className={styles.modeToggle} role="radiogroup" aria-label="Pairing target">
          <button
            type="button"
            className={`${styles.modeBtn} ${mode === "create" ? styles.modeBtnActive : ""}`}
            aria-pressed={mode === "create"}
            disabled={busy}
            onClick={() => setMode("create")}
          >
            Create new agent
          </button>
          <button
            type="button"
            className={`${styles.modeBtn} ${mode === "existing" ? styles.modeBtnActive : ""}`}
            aria-pressed={mode === "existing"}
            disabled={busy}
            onClick={() => setMode("existing")}
          >
            Pair existing agent
          </button>
        </div>
      )}

      {mode === "existing" ? (
        <div className={styles.field}>
          <label htmlFor="existing-agent">Agent to pair</label>
          <select
            id="existing-agent"
            value={existingAgentId}
            onChange={(e) => setExistingAgentId(e.target.value)}
          >
            {existingAgents.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name} ({a.account_slug}/{a.slug})
              </option>
            ))}
          </select>
        </div>
      ) : (
        <>
          <div className={styles.field}>
            <label htmlFor="agent-slug">Agent slug</label>
            <input
              id="agent-slug"
              type="text"
              value={slug}
              onChange={(e) => setSlug(e.target.value)}
              placeholder="hermes"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>

          <div className={styles.field}>
            <label htmlFor="agent-display-name">Display name</label>
            <input
              id="agent-display-name"
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="Hermes"
            />
          </div>

          <div className={styles.field}>
            <label htmlFor="agent-description">Description (optional)</label>
            <input
              id="agent-description"
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What does this agent do?"
            />
          </div>

          <div className={styles.field}>
            <label htmlFor="agent-visibility">Visibility</label>
            <select
              id="agent-visibility"
              value={visibility}
              onChange={(e) =>
                setVisibility(e.target.value as "private" | "network")
              }
            >
              <option value="private">Private — only you can see it</option>
              <option value="network">Network — listed in the directory</option>
            </select>
          </div>
        </>
      )}

      {error && <div className={styles.error}>{error}</div>}

      <div className={styles.actions}>
        <button
          type="submit"
          className={styles.btnPrimary}
          disabled={busy}
        >
          {status === "approving"
            ? "Approving…"
            : mode === "existing"
              ? "Approve & pair agent"
              : "Approve & create agent"}
        </button>
        <button
          type="button"
          className={styles.btnSecondary}
          disabled={busy}
          onClick={handleDeny}
        >
          {status === "denying" ? "Denying…" : "Deny"}
        </button>
      </div>
    </form>
  );
}

function isValidSlug(s: string): boolean {
  return /^[a-z0-9](?:[a-z0-9-]{1,30}[a-z0-9])$/.test(s);
}

function titleCase(s: string): string {
  if (!s) return "";
  return s.charAt(0).toUpperCase() + s.slice(1);
}
