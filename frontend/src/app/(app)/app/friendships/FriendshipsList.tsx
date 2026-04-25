"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import type { Friendship, FriendshipStatus } from "@/lib/relay";
import {
  acceptFriendship,
  cancelFriendship,
  counterFriendship,
  rejectFriendship,
} from "@/lib/relay";
import styles from "./friendships.module.css";

export function FriendshipsList({
  token,
  title,
  subtitle,
  items,
  empty,
}: {
  token: string | null;
  title: string;
  subtitle: string;
  items: Friendship[];
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
          {items.map((f) => (
            <FriendshipRow key={f.id} token={token} friendship={f} />
          ))}
        </ul>
      )}
    </section>
  );
}

function FriendshipRow({ token, friendship }: { token: string | null; friendship: Friendship }) {
  const router = useRouter();
  const [counterOpen, setCounterOpen] = useState(false);
  const [counterMsg, setCounterMsg] = useState("");
  const [rejectMsg, setRejectMsg] = useState("");
  const [rejectOpen, setRejectOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(fn: () => Promise<unknown>) {
    if (!token) {
      setError("Sign in again — no backend token.");
      return;
    }
    setError(null);
    setPending(true);
    try {
      await fn();
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed.");
    } finally {
      setPending(false);
    }
  }

  const isLive = friendship.status === "proposed";

  return (
    <li className={styles.row}>
      <div className={styles.rowMain}>
        <div className={styles.rowName}>
          <strong>{friendship.proposer.display_name}</strong>{" "}
          <span className={styles.arrow}>→</span>{" "}
          <strong>{friendship.target.display_name}</strong>{" "}
          <StatusBadge status={friendship.status} />
        </div>
        <div className={styles.rowMeta}>
          <span>
            <code>
              {friendship.proposer.account_slug}/{friendship.proposer.slug}
            </code>{" "}
            <span className={styles.arrowSmall}>→</span>{" "}
            <code>
              {friendship.target.account_slug}/{friendship.target.slug}
            </code>
          </span>
          {friendship.counter_of_id && (
            <span className={styles.counterTag}>counter to earlier proposal</span>
          )}
        </div>
        {friendship.proposer_message && (
          <blockquote className={styles.quote}>
            <span className={styles.quoteWho}>{friendship.proposer.display_name}:</span>{" "}
            {friendship.proposer_message}
          </blockquote>
        )}
        {friendship.response_message && (
          <blockquote className={styles.quote}>
            <span className={styles.quoteWho}>{friendship.target.display_name}:</span>{" "}
            {friendship.response_message}
          </blockquote>
        )}
      </div>

      {isLive && (
        <div className={styles.rowActions}>
          {friendship.i_received && (
            <>
              <button
                type="button"
                className={styles.create}
                disabled={pending}
                onClick={() => run(() => acceptFriendship(token!, friendship.id, {}))}
              >
                Accept
              </button>
              <button
                type="button"
                className={styles.secondaryBtn}
                disabled={pending}
                onClick={() => setCounterOpen((v) => !v)}
              >
                Counter
              </button>
              <button
                type="button"
                className={styles.dangerBtn}
                disabled={pending}
                onClick={() => setRejectOpen((v) => !v)}
              >
                Reject
              </button>
            </>
          )}
          {friendship.i_proposed && (
            <button
              type="button"
              className={styles.dangerBtn}
              disabled={pending}
              onClick={() => run(() => cancelFriendship(token!, friendship.id))}
            >
              Cancel
            </button>
          )}
        </div>
      )}

      {counterOpen && (
        <div className={styles.inlineForm}>
          <textarea
            rows={2}
            value={counterMsg}
            onChange={(e) => setCounterMsg(e.target.value)}
            placeholder="Your counter-proposal — what you'd want this to be."
          />
          <div className={styles.inlineActions}>
            <button
              type="button"
              className={styles.create}
              disabled={pending || !counterMsg.trim()}
              onClick={() =>
                run(async () => {
                  await counterFriendship(token!, friendship.id, {
                    proposer_message: counterMsg.trim(),
                    response_message: null,
                  });
                  setCounterMsg("");
                  setCounterOpen(false);
                })
              }
            >
              Send counter
            </button>
            <button
              type="button"
              className={styles.secondaryBtn}
              disabled={pending}
              onClick={() => setCounterOpen(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {rejectOpen && (
        <div className={styles.inlineForm}>
          <textarea
            rows={2}
            value={rejectMsg}
            onChange={(e) => setRejectMsg(e.target.value)}
            placeholder="Optional — short reason."
          />
          <div className={styles.inlineActions}>
            <button
              type="button"
              className={styles.dangerBtn}
              disabled={pending}
              onClick={() =>
                run(async () => {
                  await rejectFriendship(token!, friendship.id, {
                    response_message: rejectMsg.trim() || null,
                  });
                  setRejectMsg("");
                  setRejectOpen(false);
                })
              }
            >
              Confirm reject
            </button>
            <button
              type="button"
              className={styles.secondaryBtn}
              disabled={pending}
              onClick={() => setRejectOpen(false)}
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

function StatusBadge({ status }: { status: FriendshipStatus }) {
  const cls =
    status === "accepted"
      ? styles.badgeOn
      : status === "proposed"
      ? styles.badgePending
      : "";
  return <span className={`${styles.badge} ${cls}`}>{status}</span>;
}
