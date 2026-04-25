"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { deleteAgent, updateAgent, type Agent, type Visibility } from "@/lib/relay";
import styles from "../agents.module.css";

export function EditAgentForm({ token, agent }: { token: string; agent: Agent }) {
  const router = useRouter();
  const [displayName, setDisplayName] = useState(agent.display_name);
  const [description, setDescription] = useState(agent.description);
  const [endpointUrl, setEndpointUrl] = useState(agent.endpoint_url ?? "");
  const [visibility, setVisibility] = useState<Visibility>(agent.visibility);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  async function handleSave(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setSaved(false);
    setPending(true);
    try {
      await updateAgent(token, agent.id, {
        display_name: displayName.trim(),
        description: description.trim(),
        visibility,
        endpoint_url: endpointUrl.trim() || null,
      });
      setSaved(true);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't save changes.");
    } finally {
      setPending(false);
    }
  }

  async function handleDelete() {
    if (!confirm(`Delete ${agent.display_name}? Capabilities and history will go with it.`)) {
      return;
    }
    setError(null);
    setPending(true);
    try {
      await deleteAgent(token, agent.id);
      router.push("/app/agents");
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't delete.");
      setPending(false);
    }
  }

  return (
    <section className={styles.editPanel}>
      <header className={styles.formHead}>
        <h2 className={styles.sectionTitle}>Settings</h2>
        <p className={styles.formHint}>
          Slug is locked once created. Visibility flips between private
          (only members of <strong>{agent.account_display_name}</strong> see
          it) and network (everyone on this relay can discover it).
        </p>
      </header>

      <form className={styles.fields} onSubmit={handleSave}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Display name</span>
          <input
            type="text"
            required
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
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

        <label className={`${styles.field} ${styles.fieldFull}`}>
          <span className={styles.fieldLabel}>Description</span>
          <input
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>

        <label className={`${styles.field} ${styles.fieldFull}`}>
          <span className={styles.fieldLabel}>Endpoint URL</span>
          <input
            type="url"
            placeholder="https://example.com/hooks/agent"
            value={endpointUrl}
            onChange={(e) => setEndpointUrl(e.target.value)}
          />
        </label>

        <button type="submit" className={styles.create} disabled={pending}>
          {pending ? "Saving…" : "Save"}
        </button>
        <button
          type="button"
          className={styles.dangerBtn}
          onClick={handleDelete}
          disabled={pending}
        >
          Delete
        </button>
      </form>

      {saved && <div className={styles.successInline}>Saved.</div>}
      {error && <div className={styles.errorInline}>{error}</div>}
    </section>
  );
}
