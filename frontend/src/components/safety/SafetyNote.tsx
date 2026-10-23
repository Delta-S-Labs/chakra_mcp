import type { Safety } from "@/lib/relay";
import { describeSafety } from "@/lib/safety";
import styles from "./safety.module.css";

/** What message safety removed, flagged or blocked in a call, in words.
 *  Renders nothing when there's nothing to say. */
export default function SafetyNote({ safety }: { safety: Safety | undefined | null }) {
  const lines = describeSafety(safety);
  if (!lines.length) return null;
  return (
    <div className={styles.note}>
      <div className={styles.title}>Safety</div>
      <ul className={styles.lines}>
        {lines.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
    </div>
  );
}
