"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { revokeGrant, type Grant, type GrantStatus } from "@/lib/relay";
import styles from "./grants.module.css";

export function GrantsList({
  token,
  title,
  subtitle,
  items,
  empty,
}: {
  token: string | null;
  title: string;
  subtitle: string;
  items: Grant[];
  empty: string;
}) {
  return (
    <section>
      <h2 className={styles.sectionTitle}>
        {title} <span className={styles.count}>{items.length}</span>
      </h2>
      <p className={styles.formHint}>{subtitle}</p>
      {items.length === 0 ? (
        empty ? <p className={styles.empty}>{empty}</p> : null
      ) : (
        <ul className={styles.list}>
          {items.map((g) => (
            <GrantRow key={g.id} token={token} grant={g} />
          ))}
        </ul>
      )}
    </section>
  );
}

function GrantRow({ token, grant }: { token: string | null; grant: Grant }) {
  const router = useRouter();
  const [revokeOpen, setRevokeOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleRevoke() {
    if (!token) {
      setError("Sign in again — no backend token.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      await revokeGrant(token, grant.id, { reason: reason.trim() || null });
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't revoke.");
    } finally {
      setPending(false);
    }
  }

  return (
    <li className={styles.row}>
      <div className={styles.rowMain}>
        <div className={styles.rowName}>
          <strong>{grant.granter.display_name}</strong>
          <code className={styles.capCode}>{grant.capability_name}</code>
          <span className={styles.arrow}>→</span>{" "}
          <strong>{grant.grantee.display_name}</strong>{" "}
          <StatusBadge status={grant.status} />
        </div>
        <div className={styles.rowMeta}>
          <span>
            <code>
              {grant.granter.account_slug}/{grant.granter.slug}
            </code>{" "}
            <span className={styles.arrowSmall}>→</span>{" "}
            <code>
              {grant.grantee.account_slug}/{grant.grantee.slug}
            </code>
          </span>
          {grant.expires_at && (
            <span>expires {new Date(grant.expires_at).toLocaleDateString()}</span>
          )}
          {grant.revoked_at && (
            <span>revoked {new Date(grant.revoked_at).toLocaleDateString()}</span>
          )}
        </div>
        {grant.revoke_reason && (
          <blockquote className={styles.quote}>
            <span className={styles.quoteWho}>Reason:</span> {grant.revoke_reason}
          </blockquote>
        )}
      </div>

      {grant.status === "active" && grant.i_granted && (
        <div className={styles.rowActions}>
          <button
            type="button"
            className={styles.dangerBtn}
            disabled={pending}
            onClick={() => setRevokeOpen((v) => !v)}
          >
            Revoke
          </button>
        </div>
      )}

      {revokeOpen && (
        <div className={styles.inlineForm}>
          <textarea
            rows={2}
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            placeholder="Optional — short reason."
          />
          <div className={styles.inlineActions}>
            <button
              type="button"
              className={styles.dangerBtn}
              disabled={pending}
              onClick={handleRevoke}
            >
              Confirm revoke
            </button>
            <button
              type="button"
              className={styles.secondaryBtn}
              disabled={pending}
              onClick={() => setRevokeOpen(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {error && <div className={styles.errorInline}>{error}</div>}
    </li>
  );
}

function StatusBadge({ status }: { status: GrantStatus }) {
  const cls =
    status === "active"
      ? styles.badgeOn
      : status === "expired"
      ? styles.badgeNeutral
      : "";
  return <span className={`${styles.badge} ${cls}`}>{status}</span>;
}
