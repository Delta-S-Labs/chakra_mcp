"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { createAgent, type Visibility } from "@/lib/relay";
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
  const [endpointUrl, setEndpointUrl] = useState("");
  const [visibility, setVisibility] = useState<Visibility>("private");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!token) {
      setError("Sign in again — no backend token in this session.");
      return;
    }
    if (!accountId) {
      setError("Pick an account first.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      const created = await createAgent(token, {
        account_id: accountId,
        slug: slug.trim(),
        display_name: displayName.trim(),
        description: description.trim(),
        visibility,
        endpoint_url: endpointUrl.trim() || null,
      });
      router.push(`/app/agents/${created.id}`);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't create agent.");
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
          Slug must be unique within the chosen account. Endpoint URL is
          where the relay will reach this agent (Phase 1.5+ feature) —
          leave blank for now if you&apos;re just publishing capabilities.
        </p>
      </header>

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

        <label className={`${styles.field} ${styles.fieldFull}`}>
          <span className={styles.fieldLabel}>Endpoint URL (optional)</span>
          <input
            type="url"
            placeholder="https://example.com/hooks/agent"
            value={endpointUrl}
            onChange={(e) => setEndpointUrl(e.target.value)}
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
