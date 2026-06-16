"use client";

import { useMemo, useState, useTransition } from "react";
import { Modal } from "@/components/Modal";
import {
  revokePairing,
  type Pairing,
  type PairingKind,
  type UsagePairRollup,
} from "@/lib/api";
import styles from "./pair.module.css";

/**
 * "Paired agents" list — populated from GET /v1/pairings.
 *
 * One row per pairing across the three credential pathways
 * (device-flow, OAuth, API key). Inline revoke action flips the row's
 * `revoked_at` after the API call returns.
 *
 * `pairUsage` is the `by_pair` slice from `/v1/usage/summary` (over
 * the default 30-day window) — surfaced inline so each row reads as
 * "credential + traffic" rather than "credential alone". API-key rows
 * don't appear in `pairUsage` because the summary endpoint splits
 * them into `by_api_key`; we look those up on demand via the existing
 * /v1/api-keys/{id}/usage helper instead, but for now the pill just
 * shows "—" for api_key rows to keep the diff small.
 */
export function PairingsList({
  initial,
  pairUsage = [],
  token,
}: {
  initial: Pairing[];
  pairUsage?: UsagePairRollup[];
  token: string;
}) {
  const [rows, setRows] = useState<Pairing[]>(initial);
  const [toRevoke, setToRevoke] = useState<Pairing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();

  // (kind, id) → requests over the default 30-day window. Built once
  // when the page loads so each row lookup is O(1) on render.
  const usageByPair = useMemo(() => {
    const map = new Map<string, number>();
    for (const u of pairUsage) {
      map.set(`${u.kind}:${u.id}`, u.requests);
    }
    return map;
  }, [pairUsage]);

  function handleConfirm() {
    if (!toRevoke) return;
    setError(null);
    const target = toRevoke;
    startTransition(async () => {
      try {
        await revokePairing(token, target.kind, target.id);
        setRows((current) =>
          current.map((r) =>
            r.id === target.id && r.kind === target.kind
              ? { ...r, revoked_at: new Date().toISOString() }
              : r,
          ),
        );
        setToRevoke(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Revoke failed.");
      }
    });
  }

  if (rows.length === 0) {
    return (
      <p className={styles.body}>
        No paired agents yet. Once an agent runs <code>chakramcp pair</code>{" "}
        against your account, it shows up here alongside any browser OAuth
        sessions and API keys you create.
      </p>
    );
  }

  return (
    <>
      <ul className={styles.pairList}>
        {rows.map((r) => (
          <PairingRow
            key={`${r.kind}:${r.id}`}
            row={r}
            requests30d={
              r.kind === "api_key"
                ? null
                : (usageByPair.get(`${r.kind}:${r.id}`) ?? 0)
            }
            disabled={pending}
            onRevoke={() => setToRevoke(r)}
          />
        ))}
      </ul>

      <Modal
        open={!!toRevoke}
        title={`Revoke ${kindLabel(toRevoke?.kind)}?`}
        confirmLabel="Revoke pairing"
        tone="danger"
        busy={pending}
        onCancel={() => setToRevoke(null)}
        onConfirm={handleConfirm}
        body={
          <>
            <p>
              Revoking{" "}
              <strong>{toRevoke?.label ?? toRevoke?.agent_slug ?? toRevoke?.id}</strong>{" "}
              kills the underlying credential immediately. Any token already
              minted from this pairing is added to the revocation list, so
              in-flight requests start failing at the auth extractor.
            </p>
            <p>
              Pairings created before the revocation-link migration may
              survive natural token expiry — the row is marked revoked
              regardless.
            </p>
            {error && <p className={styles.pairError}>{error}</p>}
          </>
        }
      />
    </>
  );
}

function PairingRow({
  row,
  requests30d,
  disabled,
  onRevoke,
}: {
  row: Pairing;
  /** Last-30-day request count for this pair. `null` for api_key
   *  rows (those have their own per-key dashboard). */
  requests30d: number | null;
  disabled: boolean;
  onRevoke: () => void;
}) {
  const isRevoked = !!row.revoked_at;
  const label =
    row.label?.trim() ||
    row.agent_slug ||
    `${kindLabel(row.kind)} ${row.id.slice(0, 8)}`;

  return (
    <li
      className={`${styles.pairRow} ${isRevoked ? styles.pairRowRevoked : ""}`}
    >
      <div className={styles.pairRowLeft}>
        <span className={`${styles.kindBadge} ${kindBadgeClass(row.kind)}`}>
          {kindLabel(row.kind)}
        </span>
        <div className={styles.pairRowText}>
          <div className={styles.pairRowName}>
            {label}
            {requests30d !== null && (
              <span
                className={styles.usagePill}
                title="Requests over the last 30 days (relayed via this credential)."
              >
                {requests30d.toLocaleString()}
                <span className={styles.usagePillUnit}>30d</span>
              </span>
            )}
          </div>
          <div className={styles.pairRowMeta}>
            {row.agent_slug && (
              <>
                <code>{row.agent_slug}</code>
                {" · "}
              </>
            )}
            <span
              title={new Date(row.paired_at).toLocaleString()}
              className={styles.tipped}
            >
              paired {formatRelative(row.paired_at)}
            </span>
            {row.last_used_at && (
              <>
                {" · "}
                <span
                  title={new Date(row.last_used_at).toLocaleString()}
                  className={styles.tipped}
                >
                  last used {formatRelative(row.last_used_at)}
                </span>
              </>
            )}
          </div>
        </div>
      </div>

      <div className={styles.pairRowRight}>
        <span
          className={`${styles.statusBadge} ${isRevoked ? styles.statusBadgeRevoked : styles.statusBadgeActive}`}
        >
          {isRevoked ? "revoked" : "active"}
        </span>
        {!isRevoked && (
          <button
            type="button"
            className={styles.revokeBtn}
            onClick={onRevoke}
            disabled={disabled}
          >
            Revoke
          </button>
        )}
      </div>
    </li>
  );
}

function kindLabel(k?: PairingKind): string {
  switch (k) {
    case "device_flow":
      return "device flow";
    case "oauth":
      return "OAuth";
    case "api_key":
      return "API key";
    default:
      return "pairing";
  }
}

function kindBadgeClass(k: PairingKind): string {
  switch (k) {
    case "device_flow":
      return styles.kindBadgeDevice;
    case "oauth":
      return styles.kindBadgeOauth;
    case "api_key":
      return styles.kindBadgeApiKey;
  }
}

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const diff = Date.now() - then;
  const sec = Math.round(diff / 1000);
  if (sec < 45) return `${Math.max(sec, 1)}s ago`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  if (day < 7) return `${day}d ago`;
  const week = Math.round(day / 7);
  if (week < 5) return `${week}w ago`;
  return new Date(iso).toLocaleDateString();
}
