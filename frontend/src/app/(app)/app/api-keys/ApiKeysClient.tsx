"use client";

import { useState, useTransition } from "react";
import { createApiKey, revokeApiKey, type ApiKey } from "@/lib/api";
import styles from "./api-keys.module.css";

type Props = {
  initial: ApiKey[];
  backendError: string | null;
  token: string | null;
};

export function ApiKeysClient({ initial, backendError, token }: Props) {
  const [keys, setKeys] = useState<ApiKey[]>(initial);
  const [name, setName] = useState("");
  const [days, setDays] = useState<string>("90");
  const [error, setError] = useState<string | null>(backendError);
  const [justCreated, setJustCreated] = useState<{ name: string; plaintext: string } | null>(null);
  const [pending, startTransition] = useTransition();

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!token) {
      setError("No backend token in session - sign in again.");
      return;
    }
    setError(null);
    setJustCreated(null);
    const expires_in_days = days.trim() === "" ? null : Number(days);
    if (expires_in_days !== null && (Number.isNaN(expires_in_days) || expires_in_days < 1)) {
      setError("Expiration must be a positive number of days, or empty for never.");
      return;
    }

    try {
      const result = await createApiKey(token, {
        name: name.trim() || "Untitled",
        expires_in_days: expires_in_days as number | null,
      });
      setKeys((current) => [result.api_key, ...current]);
      setJustCreated({ name: result.api_key.name, plaintext: result.plaintext });
      setName("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create key.");
    }
  }

  function handleRevoke(id: string) {
    if (!token) {
      setError("No backend token in session - sign in again.");
      return;
    }
    if (!confirm("Revoke this key? Apps using it will stop working immediately.")) return;
    startTransition(async () => {
      try {
        await revokeApiKey(token, id);
        setKeys((current) =>
          current.map((k) => (k.id === id ? { ...k, revoked_at: new Date().toISOString() } : k)),
        );
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to revoke key.");
      }
    });
  }

  const active = keys.filter((k) => !k.revoked_at);
  const revoked = keys.filter((k) => k.revoked_at);

  return (
    <div className={styles.panel}>
      {error && <div className={styles.error}>{error}</div>}

      {justCreated && (
        <div className={styles.created}>
          <div className={styles.createdHead}>
            New key for <strong>{justCreated.name}</strong> - copy now, won&apos;t show again.
          </div>
          <code className={styles.createdValue}>{justCreated.plaintext}</code>
          <button
            type="button"
            className={styles.copyBtn}
            onClick={() => navigator.clipboard.writeText(justCreated.plaintext)}
          >
            Copy
          </button>
        </div>
      )}

      <form className={styles.form} onSubmit={handleCreate}>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Name</span>
          <input
            type="text"
            placeholder="e.g. Local CLI · MacBook"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
        </label>
        <label className={styles.field}>
          <span className={styles.fieldLabel}>Expires in (days)</span>
          <input
            type="text"
            placeholder="90 - empty for never"
            value={days}
            onChange={(e) => setDays(e.target.value)}
          />
        </label>
        <button type="submit" className={styles.create} disabled={pending}>
          Create key
        </button>
      </form>

      <h2 className={styles.sectionTitle}>Active</h2>
      {active.length === 0 ? (
        <p className={styles.empty}>No active keys yet.</p>
      ) : (
        <ul className={styles.list}>
          {active.map((k) => (
            <li key={k.id} className={styles.row}>
              <div className={styles.rowMain}>
                <div className={styles.rowName}>{k.name}</div>
                <div className={styles.rowMeta}>
                  <code>{k.prefix}…</code>
                  {" · "}
                  {k.expires_at
                    ? `expires ${new Date(k.expires_at).toLocaleDateString()}`
                    : "never expires"}
                  {k.last_used_at && ` · last used ${new Date(k.last_used_at).toLocaleString()}`}
                </div>
              </div>
              <button
                type="button"
                className={styles.revoke}
                onClick={() => handleRevoke(k.id)}
                disabled={pending}
              >
                Revoke
              </button>
            </li>
          ))}
        </ul>
      )}

      {revoked.length > 0 && (
        <>
          <h2 className={styles.sectionTitle}>Revoked</h2>
          <ul className={styles.list}>
            {revoked.map((k) => (
              <li key={k.id} className={`${styles.row} ${styles.rowRevoked}`}>
                <div className={styles.rowMain}>
                  <div className={styles.rowName}>{k.name}</div>
                  <div className={styles.rowMeta}>
                    <code>{k.prefix}…</code>
                    {" · revoked "}
                    {k.revoked_at && new Date(k.revoked_at).toLocaleDateString()}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
