"use client";

import { useState } from "react";
import {
  approveDeviceSession,
  denyDeviceSession,
  ApiClientError,
} from "@/lib/api";
import styles from "./onboard.module.css";

/**
 * Consent UI for an in-flight device-flow pairing session.
 *
 * On Approve: POST /oauth/device-approve → backend creates the agent
 * record and binds it to the device_code. The agent's next /oauth/token
 * poll returns the access token.
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
}: {
  token: string;
  userCode: string;
  slugHint: string;
  displayNameHint: string;
  descriptionHint: string;
}) {
  const [slug, setSlug] = useState(slugHint);
  const [displayName, setDisplayName] = useState(
    displayNameHint || titleCase(slugHint),
  );
  const [description, setDescription] = useState(descriptionHint);
  const [visibility, setVisibility] = useState<"private" | "network">(
    "private",
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

    setStatus("approving");
    try {
      const res = await approveDeviceSession(token, {
        user_code: userCode,
        agent_slug: slug.trim(),
        agent_display_name: displayName.trim(),
        agent_description: description.trim() || undefined,
        agent_visibility: visibility,
      });
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
          . You can manage it from <a href="/app">your dashboard</a>.
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

  return (
    <form onSubmit={handleApprove}>
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

      {error && <div className={styles.error}>{error}</div>}

      <div className={styles.actions}>
        <button
          type="submit"
          className={styles.btnPrimary}
          disabled={status === "approving" || status === "denying"}
        >
          {status === "approving" ? "Approving…" : "Approve & create agent"}
        </button>
        <button
          type="button"
          className={styles.btnSecondary}
          disabled={status === "approving" || status === "denying"}
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
