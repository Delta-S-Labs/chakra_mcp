"use client";

import { useState } from "react";
import styles from "./DinnerDemo.module.css";

/**
 * (B) Interactive — Alice & Bob pick dinner.
 *
 * The load-bearing UX beat: directional grants, visible.
 * Each agent declares what it WILL share and what it WON'T.
 * The user runs the negotiation, sees three candidates, picks one.
 * The final receipt shows exactly what was shared vs. withheld.
 */

type Pref = { label: string; value: string };

type Agent = {
  name: string;
  theme: "coral" | "butter" | "lime" | "ink";
  willShare: Pref[];
  wontShare: Pref[];
};

const alice: Agent = {
  name: "Alice",
  theme: "coral",
  willShare: [
    { label: "Dietary", value: "vegetarian" },
    { label: "Price cap", value: "$$" },
    { label: "Neighborhood", value: "Mission / Noe" },
  ],
  wontShare: [
    { label: "Calendar", value: "held private" },
    { label: "Location history", value: "held private" },
    { label: "Past restaurants", value: "held private" },
  ],
};

const bob: Agent = {
  name: "Bob",
  theme: "butter",
  willShare: [
    { label: "Dietary", value: "no shellfish" },
    { label: "Price cap", value: "$$$" },
    { label: "Neighborhood", value: "Mission / SoMa" },
  ],
  wontShare: [
    { label: "Calendar", value: "held private" },
    { label: "Location history", value: "held private" },
    { label: "Past restaurants", value: "held private" },
  ],
};

type Restaurant = {
  id: string;
  name: string;
  neighborhood: string;
  price: string;
  score: number;
  match: string[];
};

const candidates: Restaurant[] = [
  {
    id: "shizen",
    name: "Shizen",
    neighborhood: "Mission",
    price: "$$",
    score: 92,
    match: ["vegetarian-friendly", "no-shellfish-safe", "in both neighborhoods"],
  },
  {
    id: "flour-water",
    name: "Flour + Water",
    neighborhood: "Mission",
    price: "$$$",
    score: 87,
    match: ["vegetarian options", "no-shellfish-safe", "in both neighborhoods"],
  },
  {
    id: "al-carajo",
    name: "Al Carajo",
    neighborhood: "Noe Valley",
    price: "$$",
    score: 82,
    match: ["vegetarian options", "no-shellfish-safe", "closer to Alice"],
  },
];

type Step = "idle" | "negotiating" | "candidates" | "picked";

export default function DinnerDemo() {
  const [step, setStep] = useState<Step>("idle");
  const [picked, setPicked] = useState<Restaurant | null>(null);

  function runNegotiation() {
    setStep("negotiating");
    window.setTimeout(() => setStep("candidates"), 1600);
  }

  function pickRestaurant(r: Restaurant) {
    setPicked(r);
    setStep("picked");
  }

  function reset() {
    setPicked(null);
    setStep("idle");
  }

  return (
    <div className={styles.root}>
      <div className={styles.stage}>
        <AgentPane agent={alice} active={step === "negotiating"} />
        <RelayLane step={step} />
        <AgentPane agent={bob} active={step === "negotiating"} flip />
      </div>

      {step === "idle" && (
        <div className={styles.control}>
          <button className={styles.runButton} type="button" onClick={runNegotiation}>
            Run the negotiation →
          </button>
          <p className={styles.controlNote}>
            Click to watch the two agents exchange <em>only</em> what each side will share.
            Calendars, location history, and past restaurants never leave the device.
          </p>
        </div>
      )}

      {step === "negotiating" && (
        <div className={styles.control}>
          <div className={styles.negotiating}>
            <span className={styles.dot} />
            <span>Agents handshaking · matching preferences · filtering candidates</span>
          </div>
        </div>
      )}

      {step === "candidates" && (
        <div className={styles.candidates}>
          <div className={styles.candidatesHead}>Three candidates. Alice picks.</div>
          <div className={styles.candidateGrid}>
            {candidates.map((r) => (
              <button
                key={r.id}
                type="button"
                className={styles.candidate}
                onClick={() => pickRestaurant(r)}
              >
                <div className={styles.candidateScore}>{r.score}</div>
                <div className={styles.candidateName}>{r.name}</div>
                <div className={styles.candidateMeta}>
                  {r.neighborhood} · {r.price}
                </div>
                <ul className={styles.candidateReasons}>
                  {r.match.map((m) => (
                    <li key={m}>{m}</li>
                  ))}
                </ul>
                <div className={styles.candidatePick}>Pick</div>
              </button>
            ))}
          </div>
        </div>
      )}

      {step === "picked" && picked && (
        <div className={styles.receipt}>
          <div className={styles.receiptHead}>
            <div>
              <div className={styles.receiptEyebrow}>Grant receipt</div>
              <div className={styles.receiptTitle}>
                {picked.name} · {picked.neighborhood} · {picked.price}
              </div>
            </div>
            <button type="button" className={styles.restart} onClick={reset}>
              Restart ↺
            </button>
          </div>

          <div className={styles.receiptGrid}>
            <div className={styles.receiptCol}>
              <div className={styles.receiptColHead}>
                <span className={`${styles.colDot} ${styles.colDotCoral}`} />
                Alice shared
              </div>
              <ul>
                {alice.willShare.map((p) => (
                  <li key={p.label}>
                    <span className={styles.tick}>✓</span>
                    <strong>{p.label}:</strong> {p.value}
                  </li>
                ))}
              </ul>
              <div className={styles.receiptColHead}>
                <span className={`${styles.colDot} ${styles.colDotMuted}`} />
                Alice withheld
              </div>
              <ul>
                {alice.wontShare.map((p) => (
                  <li key={p.label} className={styles.withheld}>
                    <span className={styles.cross}>✕</span>
                    <strong>{p.label}:</strong> {p.value}
                  </li>
                ))}
              </ul>
            </div>

            <div className={styles.receiptCol}>
              <div className={styles.receiptColHead}>
                <span className={`${styles.colDot} ${styles.colDotButter}`} />
                Bob shared
              </div>
              <ul>
                {bob.willShare.map((p) => (
                  <li key={p.label}>
                    <span className={styles.tick}>✓</span>
                    <strong>{p.label}:</strong> {p.value}
                  </li>
                ))}
              </ul>
              <div className={styles.receiptColHead}>
                <span className={`${styles.colDot} ${styles.colDotMuted}`} />
                Bob withheld
              </div>
              <ul>
                {bob.wontShare.map((p) => (
                  <li key={p.label} className={styles.withheld}>
                    <span className={styles.cross}>✕</span>
                    <strong>{p.label}:</strong> {p.value}
                  </li>
                ))}
              </ul>
            </div>
          </div>

          <div className={styles.receiptFoot}>
            Directional grants kept each side&apos;s private data local. The network saw only
            the public fields each agent chose to share for this one negotiation.
          </div>
        </div>
      )}
    </div>
  );
}

/* ——— Agent pane ——— */
function AgentPane({
  agent,
  active,
  flip,
}: {
  agent: Agent;
  active: boolean;
  flip?: boolean;
}) {
  return (
    <div className={`${styles.agent} ${styles[`agent--${agent.theme}`]} ${flip ? styles.agentFlip : ""} ${active ? styles.agentActive : ""}`}>
      <div className={styles.agentHead}>
        <span className={`${styles.agentDot} ${styles[`agentDot--${agent.theme}`]}`} />
        <span>{agent.name}&apos;s agent</span>
      </div>

      <div className={styles.sectionLabel}>Will share</div>
      <ul className={styles.sharedList}>
        {agent.willShare.map((p) => (
          <li key={p.label}>
            <strong>{p.label}</strong>
            <span>{p.value}</span>
          </li>
        ))}
      </ul>

      <div className={styles.sectionLabel}>Won&apos;t share</div>
      <ul className={styles.withheldList}>
        {agent.wontShare.map((p) => (
          <li key={p.label}>
            <span className={styles.lock} aria-hidden="true">🔒</span>
            <strong>{p.label}</strong>
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ——— Relay lane (center, shows the handshake) ——— */
function RelayLane({ step }: { step: Step }) {
  return (
    <div className={`${styles.lane} ${styles[`lane--${step}`]}`}>
      <div className={styles.laneLabel}>Relay</div>
      <div className={styles.laneArrows} aria-hidden="true">
        <span className={styles.arrow}>→</span>
        <span className={styles.arrow}>←</span>
      </div>
      <div className={styles.laneCheckList}>
        <div>Friendship ✓</div>
        <div>Scope · preferences only</div>
        <div>Audit on</div>
      </div>
    </div>
  );
}
