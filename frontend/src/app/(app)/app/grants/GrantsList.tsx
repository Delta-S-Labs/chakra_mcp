"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { invoke, revokeGrant, type Grant, type GrantStatus, type InvokeResponse } from "@/lib/relay";
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
  const [invokeOpen, setInvokeOpen] = useState(false);
  const [invokeInput, setInvokeInput] = useState("{}");
  const [invokeResult, setInvokeResult] = useState<InvokeResponse | null>(null);
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

  async function handleInvoke() {
    if (!token) {
      setError("Sign in again — no backend token.");
      return;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(invokeInput);
    } catch {
      setError("Input must be valid JSON.");
      return;
    }
    setError(null);
    setInvokeResult(null);
    setPending(true);
    try {
      const resp = await invoke(token, {
        grant_id: grant.id,
        grantee_agent_id: grant.grantee.id,
        input: parsed,
      });
      setInvokeResult(resp);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invoke failed.");
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

      {grant.status === "active" && (grant.i_granted || grant.i_received) && (
        <div className={styles.rowActions}>
          {grant.i_received && (
            <button
              type="button"
              className={styles.create}
              disabled={pending}
              onClick={() => setInvokeOpen((v) => !v)}
            >
              Invoke
            </button>
          )}
          {grant.i_granted && (
            <button
              type="button"
              className={styles.dangerBtn}
              disabled={pending}
              onClick={() => setRevokeOpen((v) => !v)}
            >
              Revoke
            </button>
          )}
        </div>
      )}

      {invokeOpen && (
        <div className={styles.inlineForm}>
          <div className={styles.formHint}>
            Send JSON to <strong>{grant.granter.display_name}</strong>&apos;s{" "}
            <code>{grant.capability_name}</code> webhook. The relay HMAC-signs
            the payload.
          </div>
          <textarea
            rows={3}
            value={invokeInput}
            onChange={(e) => setInvokeInput(e.target.value)}
            placeholder='{"key":"value"}'
          />
          <div className={styles.inlineActions}>
            <button
              type="button"
              className={styles.create}
              disabled={pending}
              onClick={handleInvoke}
            >
              {pending ? "Sending…" : "Send"}
            </button>
            <button
              type="button"
              className={styles.secondaryBtn}
              disabled={pending}
              onClick={() => {
                setInvokeOpen(false);
                setInvokeResult(null);
              }}
            >
              Close
            </button>
          </div>
          {invokeResult && (
            <pre className={styles.invokeResult}>
              {JSON.stringify(invokeResult, null, 2)}
            </pre>
          )}
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
