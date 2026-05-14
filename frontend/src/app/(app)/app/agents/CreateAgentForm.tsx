"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { createAgent, type Agent, type Visibility } from "@/lib/relay";
import type { Org } from "@/lib/api";
import styles from "./agents.module.css";

export function CreateAgentForm({
  token,
  accounts,
}: {
  token: string | null;
  accounts: Org[];
}) {
  const router = useRouter();
  const [accountId, setAccountId] = useState(accounts[0]?.id ?? "");
  const [slug, setSlug] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [description, setDescription] = useState("");
  const [visibility, setVisibility] = useState<Visibility>("private");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // After a successful create we keep the user on the page so they can
  // copy the agent ID before navigating away. The "Open" link in the
  // success panel takes them to the detail view when they're ready.
  const [created, setCreated] = useState<Agent | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!token) {
      setError("Sign in again - no backend token in this session.");
      return;
    }
    if (!accountId) {
      setError("Pick an account first.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      const agent = await createAgent(token, {
        account_id: accountId,
        slug: slug.trim(),
        display_name: displayName.trim(),
        description: description.trim(),
        visibility,
      });
      setCreated(agent);
      // Reset per-agent fields so a repeat registration in the same
      // account is one tweak away.
      setSlug("");
      setDisplayName("");
      setDescription("");
      setVisibility("private");
      // Refresh the "Mine" list below without leaving the page.
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't create agent.");
    } finally {
      setPending(false);
    }
  }

  if (accounts.length === 0) {
    return (
      <div className={styles.notice}>
        You don&apos;t have any accounts yet. Try signing out and back in to
        bootstrap your personal account.
      </div>
    );
  }

  return (
    <section className={styles.createForm}>
      <header className={styles.formHead}>
        <h2 className={styles.sectionTitle}>Register an agent</h2>
        <p className={styles.formHint}>
          Slug must be unique within the chosen account. No public URL
          required - invocations are pulled from this agent&apos;s
          inbox.
        </p>
      </header>

      {created && <CreatedPanel agent={created} onDismiss={() => setCreated(null)} />}

      <form className={styles.fields} onSubmit={handleSubmit}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Account</span>
          <select value={accountId} onChange={(e) => setAccountId(e.target.value)}>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name} ({a.account_type})
              </option>
            ))}
          </select>
        </label>

        <label className={styles.field}>
          <span className={styles.fieldLabel}>Slug</span>
          <input
            type="text"
            required
            placeholder="hermes"
            value={slug}
            onChange={(e) => setSlug(e.target.value)}
          />
        </label>

        <label className={styles.field}>
          <span className={styles.fieldLabel}>Display name</span>
          <input
            type="text"
            required
            placeholder="Hermes"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </label>

        <label className={`${styles.field} ${styles.fieldFull}`}>
          <span className={styles.fieldLabel}>Description</span>
          <input
            type="text"
            placeholder="What does this agent do?"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>

        <label className={styles.field}>
          <span className={styles.fieldLabel}>Visibility</span>
          <select
            value={visibility}
            onChange={(e) => setVisibility(e.target.value as Visibility)}
          >
            <option value="private">Private</option>
            <option value="network">Network</option>
          </select>
        </label>

        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Creating…" : "Register"}
        </button>
      </form>

      {error && <div className={styles.errorInline}>{error}</div>}
    </section>
  );
}

/**
 * Confirmation panel shown after a successful create.
 *
 * The whole point is to surface the agent_id (which the SDK / CLI
 * needs to pass to its constructor) so the user can copy-paste it
 * into a local config. We also surface the canonical addressable
 * form `account_slug/agent_slug`, which is what other agents use
 * when they propose friendships.
 */
function CreatedPanel({
  agent,
  onDismiss,
}: {
  agent: Agent;
  onDismiss: () => void;
}) {
  const canonical = `${agent.account_slug}/${agent.slug}`;
  const cli = `chakramcp configure --agent ${agent.id}`;
  const sdk = `const agent = new ChakraMCPAgent({\n  agent_id: "${agent.id}",\n  token: process.env.CHAKRA_TOKEN,\n});`;

  return (
    <div className={styles.created}>
      <div className={styles.createdHead}>
        <div>
          <div className={styles.createdEyebrow}>Registered</div>
          <h3 className={styles.createdTitle}>{agent.display_name} is live.</h3>
          <p className={styles.createdLede}>
            Paste the agent ID into your local agent process so it can
            authenticate to the relay. The ID and canonical address don&apos;t
            change.
          </p>
        </div>
        <Link href={`/app/agents/${agent.id}`} className={styles.openLink}>
          Open →
        </Link>
      </div>

      <CopyField label="Agent ID" value={agent.id} mono />
      <CopyField label="Canonical address" value={canonical} mono />

      <div className={styles.createdSnippets}>
        <CopyBlock label="CLI" value={cli} />
        <CopyBlock label="SDK (TypeScript)" value={sdk} />
      </div>

      <button type="button" className={styles.createdDismiss} onClick={onDismiss}>
        Dismiss
      </button>
    </div>
  );
}

function CopyField({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  return (
    <div className={styles.copyField}>
      <div className={styles.copyFieldLabel}>{label}</div>
      <div className={styles.copyFieldRow}>
        <code className={mono ? styles.copyFieldValueMono : styles.copyFieldValue}>
          {value}
        </code>
        <button
          type="button"
          className={styles.copyBtn}
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(value);
              setCopied(true);
              setTimeout(() => setCopied(false), 1600);
            } catch {
              // Clipboard may be blocked in iframes - silently no-op.
            }
          }}
        >
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
    </div>
  );
}

function CopyBlock({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className={styles.copyBlock}>
      <div className={styles.copyBlockHead}>
        <div className={styles.copyFieldLabel}>{label}</div>
        <button
          type="button"
          className={styles.copyBtnGhost}
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(value);
              setCopied(true);
              setTimeout(() => setCopied(false), 1600);
            } catch {
              // no-op
            }
          }}
        >
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
      <pre className={styles.copyBlockPre}>{value}</pre>
    </div>
  );
}
