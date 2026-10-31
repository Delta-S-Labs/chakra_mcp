"use client";

import { useState } from "react";
import styles from "./story.module.css";

type Answer = "week" | "once" | "declined";

const OUTCOMES: Record<Answer, { title: string; body: string }> = {
  week: {
    title: "Approved for 7 days.",
    body: "ana/concierge can use charge_deposit up to 3 times in the next 7 days. You can take it back whenever you like.",
  },
  once: {
    title: "Approved for this request.",
    body: "The deposit goes through. The next request comes back to you.",
  },
  declined: {
    title: "Declined.",
    body: "ana/concierge gets your note: we take deposits at the door.",
  },
};

/**
 * The landing's live tile: a request waiting for the owner's consent.
 * The visitor answers it and the tile shows what the relay does next.
 * Without script the waiting request still reads in full; the buttons
 * just do nothing.
 */
export default function ConsentTile({ className = "" }: { className?: string }) {
  const [answer, setAnswer] = useState<Answer | null>(null);
  const outcome = answer ? OUTCOMES[answer] : null;

  return (
    <section className={`${styles.tile} ${styles.tileCoral} ${className}`} aria-labelledby="ask-title">
      <h3 id="ask-title" className={styles.h3}>
        Some requests wait for you
      </h3>
      <div data-ui className={styles.frag}>
        <div className={styles.fragHead}>
          <span className={styles.fragTitle}>Waiting for you</span>
          <span className={styles.fragMeta}>2 minutes ago</span>
        </div>
        <p className={styles.ask}>
          <span className={styles.who}>ana/concierge</span> wants to use{" "}
          <span className={styles.who}>bob/venue-bot</span>&apos;s charge_deposit.
        </p>
        <dl className={styles.terms} style={{ marginTop: 12 }}>
          <dt>Purpose</dt>
          <dd>Deposit for Friday&apos;s table, VB-2291</dd>
          <dt>Amount</dt>
          <dd>$60.00</dd>
          <dt>Asked for</dt>
          <dd>7 days, up to 3 requests</dd>
        </dl>
      </div>
      <div aria-live="polite">
        {outcome ? (
          <div className={styles.outcome}>
            <b>{outcome.title}</b>
            {outcome.body}
          </div>
        ) : null}
      </div>
      <div className={styles.choices}>
        {answer ? (
          <button type="button" className={styles.choice} onClick={() => setAnswer(null)}>
            Ask me again
          </button>
        ) : (
          <>
            <button
              type="button"
              className={`${styles.choice} ${styles.choicePrimary}`}
              onClick={() => setAnswer("week")}
            >
              Approve for 7 days
            </button>
            <button type="button" className={styles.choice} onClick={() => setAnswer("once")}>
              Just this once
            </button>
            <button type="button" className={styles.choice} onClick={() => setAnswer("declined")}>
              Decline
            </button>
          </>
        )}
      </div>
    </section>
  );
}
