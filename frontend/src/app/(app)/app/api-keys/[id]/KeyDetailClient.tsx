"use client";

import { useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { Modal } from "@/components/Modal";
import {
  revokeApiKey,
  rotateApiKey,
  type ApiKey,
  type ApiKeyUsage,
} from "@/lib/api";
import { UsageCharts } from "./UsageCharts";
import styles from "./detail.module.css";

/**
 * Client island for the api-key detail page. Owns the rotate/revoke
 * buttons (with confirmation modals + one-time plaintext reveal on
 * rotate) and the usage charts.
 */
export function KeyDetailClient({
  apiKey,
  initialUsage,
  token,
}: {
  apiKey: ApiKey;
  initialUsage: ApiKeyUsage | null;
  token: string;
}) {
  const router = useRouter();
  const [key, setKey] = useState<ApiKey>(apiKey);
  const [error, setError] = useState<string | null>(null);
  const [openRotate, setOpenRotate] = useState(false);
  const [openRevoke, setOpenRevoke] = useState(false);
  const [justRotated, setJustRotated] = useState<{
    name: string;
    plaintext: string;
    newId: string;
  } | null>(null);
  const [pending, startTransition] = useTransition();

  const isRevoked = !!key.revoked_at;

  function handleRotate() {
    setError(null);
    startTransition(async () => {
      try {
        const result = await rotateApiKey(token, key.id);
        // The old row is now revoked; the new row has a different id.
        // Show the plaintext, then offer a one-click jump to the new
        // key's detail page.
        setJustRotated({
          name: result.api_key.name,
          plaintext: result.plaintext,
          newId: result.api_key.id,
        });
        setKey((current) => ({
          ...current,
          revoked_at: new Date().toISOString(),
        }));
        setOpenRotate(false);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Rotate failed.");
      }
    });
  }

  function handleRevoke() {
    setError(null);
    startTransition(async () => {
      try {
        await revokeApiKey(token, key.id);
        setKey((current) => ({
          ...current,
          revoked_at: new Date().toISOString(),
        }));
        setOpenRevoke(false);
        router.refresh();
      } catch (err) {
        setError(err instanceof Error ? err.message : "Revoke failed.");
      }
    });
  }

  return (
    <>
      <div className={styles.actionBar}>
        <button
          type="button"
          className={styles.actionBtn}
          onClick={() => setOpenRotate(true)}
          disabled={isRevoked || pending}
          title={isRevoked ? "Revoked keys can't be rotated." : undefined}
        >
          Rotate
        </button>
        <button
          type="button"
          className={styles.actionBtnDanger}
          onClick={() => setOpenRevoke(true)}
          disabled={isRevoked || pending}
        >
          Revoke
        </button>
      </div>

      {error && <div className={styles.error}>{error}</div>}

      {justRotated && (
        <div className={styles.created}>
          <div className={styles.createdHead}>
            New key for <strong>{justRotated.name}</strong> — copy now,
            won&apos;t show again.
          </div>
          <code className={styles.createdValue}>{justRotated.plaintext}</code>
          <button
            type="button"
            className={styles.copyBtn}
            onClick={() => navigator.clipboard.writeText(justRotated.plaintext)}
          >
            Copy
          </button>
          <p className={styles.rotatedFoot}>
            The old key (this page) is now revoked. The replacement lives at{" "}
            <a href={`/app/api-keys/${justRotated.newId}`}>
              its own detail page
            </a>
            .
          </p>
        </div>
      )}

      <UsageCharts usage={initialUsage} />

      <Modal
        open={openRotate}
        title={`Rotate ${key.name}?`}
        confirmLabel="Rotate key"
        tone="danger"
        busy={pending}
        onCancel={() => setOpenRotate(false)}
        onConfirm={handleRotate}
        body={
          <p>
            Rotate <code>{key.name}</code>? The old key keeps working for 0
            seconds — you&apos;ll get a new key shown once; clients still on
            the old one will fail.
          </p>
        }
      />

      <Modal
        open={openRevoke}
        title={`Revoke ${key.name}?`}
        confirmLabel="Revoke key"
        tone="danger"
        busy={pending}
        onCancel={() => setOpenRevoke(false)}
        onConfirm={handleRevoke}
        body={
          <p>
            Revoking <code>{key.prefix}…</code> kills the credential
            immediately. Any client still using it will fail at the auth
            extractor.
          </p>
        }
      />
    </>
  );
}
