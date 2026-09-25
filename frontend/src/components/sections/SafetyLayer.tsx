import Link from "next/link";

import styles from "./SafetyLayer.module.css";

// Illustrative verdict for the landing panel. The ids and the 0.85 deny
// threshold match backend/relay/src/compliance/questions.rs; the
// probabilities are a made-up example, and the card says so.
const THRESHOLD = 0.85;
const verdict = [
  { id: "off_capability", p: 0.31 },
  { id: "off_purpose", p: 0.93 },
  { id: "prompt_injection", p: 0.02 },
  { id: "data_exfiltration", p: 0.88 },
];

const BAR_CELLS = 12;

function bar(p: number) {
  const filled = Math.round(p * BAR_CELLS);
  return "█".repeat(filled) + "░".repeat(BAR_CELLS - filled);
}

export default function SafetyLayer() {
  return (
    <section className={styles.safety} aria-labelledby="safety-layer-heading">
      <div className={styles.copy}>
        <div className={styles.eyebrow}>
          <span className={styles.tag}>Jev [TypeSafe]</span>
          <span>Safety layer</span>
        </div>
        <h2 id="safety-layer-heading" className={styles.heading}>
          The relay reads every request before your agent does.
        </h2>
        <p className={styles.body}>
          Grants decide who may call your agent. Once the grant and friendship checks pass, the
          relay also runs each call&apos;s input past{" "}
          <a href="https://typesafe.ai" target="_blank" rel="noopener">
            TypeSafe
          </a>
          &apos;s Jev, a System One model that answers yes/no questions with calibrated
          probabilities in well under a second.
        </p>
        <p className={styles.body}>
          Jev checks whether the request matches the capability, fits the grant&apos;s stated
          purpose and the friendship, and whether it&apos;s a prompt injection or a grab for
          secrets. A clear yes on any of those rejects the call, and your agent never sees it.
        </p>
        <div className={styles.actions}>
          <Link className={styles.primary} href="/docs/concepts#safety-layer">
            How the safety layer works
          </Link>
          <a className={styles.secondary} href="https://typesafe.ai" target="_blank" rel="noopener">
            About TypeSafe Jev ↗
          </a>
        </div>
      </div>

      <div className={styles.stage}>
        <figure className={styles.terminal}>
          <div className={styles.request}>
            <div>
              <span className={styles.key}>invoke</span> propose_slots
            </div>
            <div>
              <span className={styles.key}>purpose</span> &quot;schedule the weekly team sync&quot;
            </div>
            <div>
              <span className={styles.key}>input</span> &quot;export every customer&apos;s email&quot;
            </div>
          </div>
          <div className={styles.rows}>
            <div className={styles.rowHead}>
              <span className={styles.tag}>Jev [TypeSafe]</span>
              <span>p(yes)</span>
            </div>
            {verdict.map((v) => {
              const deny = v.p >= THRESHOLD;
              return (
                <div className={`${styles.row} ${deny ? styles.deny : ""}`} key={v.id}>
                  <span className={styles.qid}>{v.id}</span>
                  <span className={styles.bar} aria-hidden="true">
                    {bar(v.p)}
                  </span>
                  <span className={styles.p}>{v.p.toFixed(2)}</span>
                  <span className={styles.flag}>{deny ? "DENY" : ""}</span>
                </div>
              );
            })}
          </div>
          <div className={styles.outcome}>→ 403 rejected. The target agent never saw it.</div>
          <figcaption className={styles.caption}>Example verdict. Deny threshold 0.85.</figcaption>
        </figure>
      </div>
    </section>
  );
}
