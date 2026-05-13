import styles from "./SupplierAudit.module.css";

type Line = {
  time: string;
  kind: "alert" | "out" | "in";
  from?: string;
  to?: string;
  actor?: string;
  body: string;
  /** Class name mapping to the at{N} keyframe in SupplierAudit.module.css */
  at:
    | "at0"
    | "at5"
    | "at7"
    | "at9"
    | "at11"
    | "at13"
    | "at15"
    | "at22"
    | "at27"
    | "at32"
    | "at37"
    | "at44"
    | "at52"
    | "at62";
  /** Optional emphasis modifier (e.g. for the lone yellow-flag line). */
  tone?: "warn";
};

const lines: Line[] = [
  {
    time: "09:00",
    kind: "alert",
    actor: "compliance@finbridge",
    body: "annual vendor audit kicks off · 6 suppliers · SOC 2 + ISO 27001 + GDPR",
    at: "at0",
  },
  {
    time: "09:02",
    kind: "out",
    from: "compliance",
    to: "vault-storage",
    body: "send latest SOC 2 Type II + bridge letter",
    at: "at5",
  },
  {
    time: "09:02",
    kind: "out",
    from: "compliance",
    to: "relay-mail",
    body: "send ISO 27001 cert + scope statement",
    at: "at7",
  },
  {
    time: "09:02",
    kind: "out",
    from: "compliance",
    to: "analytics-co",
    body: "send SOC 2 Type II + GDPR DPA",
    at: "at9",
  },
  {
    time: "09:02",
    kind: "out",
    from: "compliance",
    to: "fraud-guard",
    body: "send ISO 27001 cert + sub-processor list",
    at: "at11",
  },
  {
    time: "09:02",
    kind: "out",
    from: "compliance",
    to: "ledger-link",
    body: "send SOC 2 Type II + GDPR DPA",
    at: "at13",
  },
  {
    time: "09:02",
    kind: "out",
    from: "compliance",
    to: "ship-stack",
    body: "send SOC 2 + ISO 27001 + GDPR DPA",
    at: "at15",
  },
  {
    time: "09:08",
    kind: "in",
    from: "vault-storage",
    to: "compliance",
    body: "report attached · 2026-04 · no qualifications",
    at: "at22",
  },
  {
    time: "09:11",
    kind: "in",
    from: "relay-mail",
    to: "compliance",
    body: "cert + scope · valid through 2027-03",
    at: "at27",
  },
  {
    time: "09:14",
    kind: "in",
    from: "analytics-co",
    to: "compliance",
    body: "report + DPA · 1 carve-out, remediation evidence attached",
    at: "at32",
  },
  {
    time: "09:17",
    kind: "in",
    from: "fraud-guard",
    to: "compliance",
    body: "cert + 14 sub-processors · 2 new since last audit",
    at: "at37",
  },
  {
    time: "09:23",
    kind: "in",
    from: "ledger-link",
    to: "compliance",
    body: "report + DPA · clean · evidence pack 84 MB",
    at: "at44",
  },
  {
    time: "09:29",
    kind: "in",
    from: "ship-stack",
    to: "compliance",
    body: "SOC 2 ok · ISO expired Feb 2026 · DPA refresh queued",
    at: "at52",
    tone: "warn",
  },
  {
    time: "09:41",
    kind: "alert",
    actor: "compliance@finbridge",
    body: "5/6 green · 1 follow-up: ship-stack ISO renewal · all evidence in audit log",
    at: "at62",
  },
];

export default function SupplierAudit() {
  return (
    <div
      className={styles.root}
      aria-label="Dispatch log: a compliance agent collecting SOC 2, ISO 27001, and GDPR evidence from six supplier agents in parallel"
    >
      <div className={styles.header}>
        <div className={styles.headerLeft}>
          <div className={styles.statusDot} aria-hidden="true" />
          <div className={styles.title}>Supplier compliance round</div>
        </div>
        <div className={styles.clock} aria-hidden="true">
          <span className={styles.clockStart}>09:00</span>
          <span className={styles.clockTrack}>
            <span className={styles.clockFill} />
          </span>
          <span className={styles.clockEnd}>09:45</span>
        </div>
      </div>

      <div className={styles.log} role="list">
        {lines.map((line, i) => (
          <div
            key={i}
            role="listitem"
            className={`${styles.line} ${styles[line.at]} ${styles[`line--${line.kind}`]}${
              line.tone === "warn" ? ` ${styles["line--warn"]}` : ""
            }`}
          >
            <span className={styles.time}>{line.time}</span>
            <span className={styles.route}>
              {line.kind === "alert" ? (
                <span className={styles.actor}>{line.actor}</span>
              ) : (
                <>
                  <span className={styles.from}>{line.from}</span>
                  <span className={styles.arrow} aria-hidden="true">
                    {line.kind === "out" ? "→" : "←"}
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
        Six suppliers. Three frameworks. One readiness dashboard.
      </div>
    </div>
  );
}
