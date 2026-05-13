import styles from "./TaxAgentShopping.module.css";

type Line = {
  time: string;
  kind: "alert" | "out" | "in";
  from?: string;
  to?: string;
  actor?: string;
  body: string;
  /** Class name mapping to the at{N} keyframe in TaxAgentShopping.module.css */
  at:
    | "at0"
    | "at3"
    | "at6"
    | "at9"
    | "at12"
    | "at15"
    | "at18"
    | "at22"
    | "at26"
    | "at30"
    | "at34"
    | "at38"
    | "at42"
    | "at48"
    | "at54"
    | "at62"
    | "at68"
    | "at74";
  /** Optional emphasis modifier (e.g. for the decision moment). */
  tone?: "pick";
};

const lines: Line[] = [
  {
    time: "10:00",
    kind: "alert",
    actor: "user-agent@kaustav",
    body: "tax prep needed · US options, ETH/SOL spot, intl. stocks (LSE + NSE)",
    at: "at0",
  },
  {
    time: "10:00",
    kind: "out",
    from: "user-agent",
    to: "tax-directory",
    body: "list agents with: options + crypto-cost-basis + foreign-tax-credit",
    at: "at3",
  },
  {
    time: "10:01",
    kind: "in",
    from: "tax-directory",
    to: "user-agent",
    body: "5 matches · ranked by recent client reviews",
    at: "at6",
  },
  {
    time: "10:01",
    kind: "out",
    from: "user-agent",
    to: "tax-aurora",
    body: "capabilities + price + turnaround for my brief?",
    at: "at9",
  },
  {
    time: "10:01",
    kind: "out",
    from: "user-agent",
    to: "tax-blackwell",
    body: "capabilities + price + turnaround for my brief?",
    at: "at12",
  },
  {
    time: "10:01",
    kind: "out",
    from: "user-agent",
    to: "tax-clearwater",
    body: "capabilities + price + turnaround for my brief?",
    at: "at15",
  },
  {
    time: "10:01",
    kind: "out",
    from: "user-agent",
    to: "tax-deltaco",
    body: "capabilities + price + turnaround for my brief?",
    at: "at18",
  },
  {
    time: "10:01",
    kind: "out",
    from: "user-agent",
    to: "tax-everline",
    body: "capabilities + price + turnaround for my brief?",
    at: "at22",
  },
  {
    time: "10:02",
    kind: "in",
    from: "tax-aurora",
    to: "user-agent",
    body: "$420 · 4 days · 1,200 clients · options ✓ crypto ✓ FTC ✓",
    at: "at26",
  },
  {
    time: "10:02",
    kind: "in",
    from: "tax-blackwell",
    to: "user-agent",
    body: "$650 · 2 days · 320 clients · options ✓ crypto ✓ FTC ✓",
    at: "at30",
  },
  {
    time: "10:02",
    kind: "in",
    from: "tax-clearwater",
    to: "user-agent",
    body: "$310 · 7 days · 4,100 clients · options ✓ crypto ✗ FTC ✓",
    at: "at34",
  },
  {
    time: "10:02",
    kind: "in",
    from: "tax-deltaco",
    to: "user-agent",
    body: "$520 · 3 days · 880 clients · options ✓ crypto ✓ FTC ✓",
    at: "at38",
  },
  {
    time: "10:02",
    kind: "in",
    from: "tax-everline",
    to: "user-agent",
    body: "$390 · 5 days · 2,200 clients · options ✓ crypto ✓ FTC partial",
    at: "at42",
  },
  {
    time: "10:04",
    kind: "alert",
    actor: "user-agent@kaustav",
    body: "shortlist: aurora · blackwell · deltaco · presenting",
    at: "at48",
  },
  {
    time: "10:06",
    kind: "alert",
    actor: "user@kaustav",
    body: "picks tax-aurora · grants 60-day read scope on brokerage + crypto-exchange agents",
    at: "at54",
    tone: "pick",
  },
  {
    time: "10:07",
    kind: "out",
    from: "user-agent",
    to: "brokerage",
    body: "grant: read 2025 tax-relevant transactions · until 2026-06-30 · for tax-aurora",
    at: "at62",
  },
  {
    time: "10:07",
    kind: "out",
    from: "user-agent",
    to: "crypto-exchange",
    body: "grant: read 2025 tax-relevant transactions · until 2026-06-30 · for tax-aurora",
    at: "at68",
  },
  {
    time: "10:08",
    kind: "in",
    from: "tax-aurora",
    to: "user-agent",
    body: "received scopes · prelim 1099 + 8949 + 1116 in 3 days",
    at: "at74",
  },
];

export default function TaxAgentShopping() {
  return (
    <div
      className={styles.root}
      aria-label="Dispatch log: a personal agent shopping for a tax-prep agent across five candidates, then granting scoped access to brokerage and crypto-exchange agents"
    >
      <div className={styles.header}>
        <div className={styles.headerLeft}>
          <div className={styles.statusDot} aria-hidden="true" />
          <div className={styles.title}>Tax season · pick an agent</div>
        </div>
        <div className={styles.clock} aria-hidden="true">
          <span className={styles.clockStart}>TUE 10:00</span>
          <span className={styles.clockTrack}>
            <span className={styles.clockFill} />
          </span>
          <span className={styles.clockEnd}>TUE 10:08</span>
        </div>
      </div>

      <div className={styles.log} role="list">
        {lines.map((line, i) => (
          <div
            key={i}
            role="listitem"
            className={`${styles.line} ${styles[line.at]} ${styles[`line--${line.kind}`]}${
              line.tone === "pick" ? ` ${styles["line--pick"]}` : ""
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
        Five agents pinged. Three shortlisted. One picked. No phone calls.
      </div>
    </div>
  );
}
