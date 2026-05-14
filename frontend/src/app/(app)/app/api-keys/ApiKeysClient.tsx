"use client";

import { useState, useTransition } from "react";
import Link from "next/link";
import { Modal } from "@/components/Modal";
import {
  createApiKey,
  revokeApiKey,
  rotateApiKey,
  type ApiKey,
} from "@/lib/api";
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
  const [justCreated, setJustCreated] = useState<{
    name: string;
    plaintext: string;
  } | null>(null);
  const [toRevoke, setToRevoke] = useState<ApiKey | null>(null);
  const [toRotate, setToRotate] = useState<ApiKey | null>(null);
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
    if (
      expires_in_days !== null &&
      (Number.isNaN(expires_in_days) || expires_in_days < 1)
    ) {
      setError("Expiration must be a positive number of days, or empty for never.");
      return;
    }

    try {
      const result = await createApiKey(token, {
        name: name.trim() || "Untitled",
        expires_in_days: expires_in_days as number | null,
      });
      setKeys((current) => [result.api_key, ...current]);
      setJustCreated({
        name: result.api_key.name,
        plaintext: result.plaintext,
      });
      setName("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create key.");
    }
  }

  function handleConfirmRevoke() {
    if (!token || !toRevoke) return;
    const target = toRevoke;
    setError(null);
    startTransition(async () => {
      try {
        await revokeApiKey(token, target.id);
        setKeys((current) =>
          current.map((k) =>
            k.id === target.id
              ? { ...k, revoked_at: new Date().toISOString() }
              : k,
          ),
        );
        setToRevoke(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to revoke key.");
      }
    });
  }

  function handleConfirmRotate() {
    if (!token || !toRotate) return;
    const target = toRotate;
    setError(null);
    startTransition(async () => {
      try {
        const result = await rotateApiKey(token, target.id);
        // The old key is now revoked; the new one is a fresh row.
        setKeys((current) => [
          result.api_key,
          ...current.map((k) =>
            k.id === target.id
              ? { ...k, revoked_at: new Date().toISOString() }
              : k,
          ),
        ]);
        setJustCreated({
          name: result.api_key.name,
          plaintext: result.plaintext,
        });
        setToRotate(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to rotate key.");
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
            New key for <strong>{justCreated.name}</strong> - copy now,
            won&apos;t show again.
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
            <KeyRow
              key={k.id}
              k={k}
              pending={pending}
              onRevoke={() => setToRevoke(k)}
              onRotate={() => setToRotate(k)}
            />
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
                    {" · created "}
                    {new Date(k.created_at).toLocaleDateString()}
                    {" · revoked "}
                    {k.revoked_at && new Date(k.revoked_at).toLocaleDateString()}
                  </div>
                </div>
                <Link href={`/app/api-keys/${k.id}`} className={styles.openLink}>
                  Details →
                </Link>
              </li>
            ))}
          </ul>
        </>
      )}

      <Modal
        open={!!toRevoke}
        title={`Revoke ${toRevoke?.name ?? "this key"}?`}
        confirmLabel="Revoke key"
        tone="danger"
        busy={pending}
        onCancel={() => setToRevoke(null)}
        onConfirm={handleConfirmRevoke}
        body={
          <p>
            Revoking <code>{toRevoke?.prefix}…</code> kills the credential
            immediately. Any client still using it will fail at the auth
            extractor.
          </p>
        }
      />

      <Modal
        open={!!toRotate}
        title={`Rotate ${toRotate?.name ?? "this key"}?`}
        confirmLabel="Rotate key"
        tone="danger"
        busy={pending}
        onCancel={() => setToRotate(null)}
        onConfirm={handleConfirmRotate}
        body={
          <p>
            Rotate <code>{toRotate?.name}</code>? The old key keeps working
            for 0 seconds — you&apos;ll get a new key shown once; clients
            still on the old one will fail.
          </p>
        }
      />
    </div>
  );
}

function KeyRow({
  k,
  pending,
  onRevoke,
  onRotate,
}: {
  k: ApiKey;
  pending: boolean;
  onRevoke: () => void;
  onRotate: () => void;
}) {
  return (
    <li className={styles.row}>
      <Link className={styles.rowLink} href={`/app/api-keys/${k.id}`}>
        <div className={styles.rowMain}>
          <div className={styles.rowName}>{k.name}</div>
          <div className={styles.rowMeta}>
            <code>{k.prefix}…</code>
            {" · created "}
            {new Date(k.created_at).toLocaleDateString()}
            {" · "}
            {k.expires_at
              ? `expires ${new Date(k.expires_at).toLocaleDateString()}`
              : "never expires"}
            {k.last_used_at &&
              ` · last used ${new Date(k.last_used_at).toLocaleString()}`}
          </div>
        </div>
      </Link>
      <div className={styles.rowActions}>
        <button
          type="button"
          className={styles.rotate}
          onClick={onRotate}
          disabled={pending}
        >
          Rotate
        </button>
        <button
          type="button"
          className={styles.revoke}
          onClick={onRevoke}
          disabled={pending}
        >
          Revoke
        </button>
      </div>
    </li>
  );
}
