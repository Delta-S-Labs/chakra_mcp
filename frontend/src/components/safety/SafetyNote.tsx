import type { Safety, SafetyFinding } from "@/lib/relay";
import { describeFindings, describeSafety } from "@/lib/safety";
import styles from "./safety.module.css";

/** What message safety removed, flagged or blocked, in words: an
 *  invocation's `safety`, or one call message's `findings`. Renders nothing
 *  when there's nothing to say. */
export default function SafetyNote({
  safety,
  findings,
}: {
  safety?: Safety | null;
  findings?: SafetyFinding[] | null;
}) {
  const lines = findings ? describeFindings(findings, "message") : describeSafety(safety);
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
