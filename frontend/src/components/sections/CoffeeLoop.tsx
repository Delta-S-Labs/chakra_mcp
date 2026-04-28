import styles from "./CoffeeLoop.module.css";

type Line = {
  time: string;
  kind: "alert" | "out" | "in";
  from?: string;
  to?: string;
  actor?: string;
  body: string;
  /** Class name mapping to the at{N} keyframe in CoffeeLoop.module.css */
  at: "at3" | "at15" | "at20" | "at25" | "at37" | "at45" | "at53" | "at68";
};

const lines: Line[] = [
  { time: "03:00", kind: "alert", actor: "coffee-shop@corner", body: "inventory low - croissants, milk, beans", at: "at3" },
  { time: "03:14", kind: "out", from: "coffee-shop", to: "bakery", body: "40 croissants \u00b7 20 pain au chocolat \u00b7 by 6am", at: "at15" },
  { time: "03:14", kind: "out", from: "coffee-shop", to: "produce", body: "20 L whole milk \u00b7 by 6am", at: "at20" },
  { time: "03:14", kind: "out", from: "coffee-shop", to: "mill", body: "8 kg Ethiopian medium roast \u00b7 by 6am", at: "at25" },
  { time: "03:47", kind: "in", from: "bakery", to: "coffee-shop", body: "confirmed \u00b7 eta 5:30 \u00b7 $182", at: "at37" },
  { time: "04:02", kind: "in", from: "produce", to: "coffee-shop", body: "confirmed \u00b7 eta 5:45 \u00b7 $64", at: "at45" },
  { time: "04:31", kind: "in", from: "mill", to: "coffee-shop", body: "confirmed \u00b7 eta 6:00 \u00b7 $310", at: "at53" },
  { time: "06:58", kind: "alert", actor: "coffee-shop@corner", body: "opening - three handshakes, one owner", at: "at68" },
];

export default function CoffeeLoop() {
  return (
    <div className={styles.root} aria-label="Dispatch log: four agents coordinating a coffee shop restock overnight">
      <div className={styles.header}>
        <div className={styles.headerLeft}>
          <div className={styles.statusDot} aria-hidden="true" />
          <div className={styles.title}>Night shift traffic</div>
        </div>
        <div className={styles.clock} aria-hidden="true">
          <span className={styles.clockStart}>03:00</span>
          <span className={styles.clockTrack}>
            <span className={styles.clockFill} />
          </span>
          <span className={styles.clockEnd}>07:00</span>
        </div>
      </div>

      <div className={styles.log} role="list">
        {lines.map((line, i) => (
          <div
            key={i}
            role="listitem"
            className={`${styles.line} ${styles[line.at]} ${styles[`line--${line.kind}`]}`}
          >
            <span className={styles.time}>{line.time}</span>
            <span className={styles.route}>
              {line.kind === "alert" ? (
                <span className={styles.actor}>{line.actor}</span>
              ) : (
                <>
                  <span className={styles.from}>{line.from}</span>
                  <span className={styles.arrow} aria-hidden="true">
                    {line.kind === "out" ? "\u2192" : "\u2190"}
                  </span>
                  <span className={styles.to}>{line.to}</span>
                </>
              )}
            </span>
            <span className={styles.body}>{line.body}</span>
          </div>
        ))}
      </div>

      <div className={styles.footer}>
        Three agents. One owner. Zero alarms set for 3am.
      </div>
    </div>
  );
}
