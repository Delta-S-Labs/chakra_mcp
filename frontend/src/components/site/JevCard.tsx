import { JetBrains_Mono } from "next/font/google";

import styles from "./jev.module.css";

// TypeSafe's terminal face; loaded only on pages that show a Jev card.
const mono = JetBrains_Mono({ subsets: ["latin"], weight: ["400", "500"], display: "swap" });

export type JevRow = {
  /** The question id, or a message id for a recall. */
  label: string;
  /** What the row is about, when the label alone doesn't say (a message's text). */
  text?: string;
  /** Jev's probability. */
  p: number;
  /** Mark the row; by default, rows at or past `threshold` are marked. */
  mark?: boolean;
};

type Props = {
  /** The window's title bar. */
  title?: string;
  /** What the request was, one line each: a grey key and its value. */
  lines?: { key: string; value: React.ReactNode }[];
  /** The relay's own checks before Jev reads anything. */
  checks?: { label: string; value: string }[];
  /** The heading over Jev's rows, and the column it reports. */
  head?: string;
  column?: string;
  rows: JevRow[];
  /** Where a yes acts; drawn as a tick on each row's rule. */
  threshold?: number;
  /** What the relay did with the answers. */
  outcome: React.ReactNode;
  caption?: React.ReactNode;
  /** Set it on TypeSafe's dotted pink field (off where the field is already there). */
  field?: boolean;
  className?: string;
};

/**
 * A Jev verdict drawn in TypeSafe's own visual language (typesafe.ai): a
 * terminal window on a dotted pink field, its title in pink on near-black,
 * black-inverse labels, and each probability as a square on a rule. It
 * marks every place the site talks about TypeSafe; nothing else on the
 * site looks like this. CSS only; no TypeSafe assets.
 */
export default function JevCard({
  title = "Jev [TypeSafe]",
  lines,
  checks,
  head = "Jev [TypeSafe]",
  column = "p(yes)",
  rows,
  threshold,
  outcome,
  caption,
  field = true,
  className = "",
}: Props) {
  const card = (
    <div className={styles.window}>
      <div className={styles.bar}>
        <span>{title}</span>
        <span aria-hidden="true">▪ ▪</span>
      </div>
      {(lines || checks) && (
        <div className={styles.pane}>
          {lines?.map((l) => (
            <p key={l.key} className={styles.line}>
              <span className={styles.key}>{l.key}</span>
              <span className={styles.value}>{l.value}</span>
            </p>
          ))}
          {checks && (
            <ul className={styles.checks} aria-label="The relay's checks">
              {checks.map((c) => (
                <li key={c.label}>
                  <span>{c.label}</span>
                  <span>{c.value}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      <div className={styles.pane}>
        <p className={styles.rowsHead}>
          <span className={styles.chip}>{head}</span>
          <span className={styles.key}>{column}</span>
        </p>
        <ul
          className={styles.rows}
          style={threshold === undefined ? undefined : ({ "--t": threshold } as React.CSSProperties)}
        >
          {rows.map((r) => {
            const marked = r.mark ?? (threshold !== undefined && r.p >= threshold);
            return (
              <li key={r.label} className={marked ? styles.marked : undefined}>
                <span className={styles.rowTop}>
                  <span className={styles.chip}>{r.label}</span>
                  {r.text && <span className={styles.rowText}>{r.text}</span>}
                  <span className={styles.p}>{r.p.toFixed(2)}</span>
                </span>
                <span
                  className={`${styles.rule} ${threshold === undefined ? "" : styles.ruleTick}`}
                  aria-hidden="true"
                >
                  <span className={styles.square} style={{ "--p": r.p } as React.CSSProperties} />
                </span>
              </li>
            );
          })}
        </ul>
        <div className={styles.dots} aria-hidden="true" />
      </div>
      <p className={styles.outcome}>{outcome}</p>
    </div>
  );

  return (
    <figure className={`${mono.className} ${styles.figure} ${field ? styles.field : ""} ${className}`}>
      {card}
      {caption && <figcaption className={styles.caption}>{caption}</figcaption>}
    </figure>
  );
}

/** TypeSafe's dotted pink field, for a tile or section that holds a Jev card. */
export const jevField = styles.field;
