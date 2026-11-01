"use client";

import { useEffect, useRef, useState } from "react";
import h from "./RelayHero.module.css";

/**
 * The landing's live picture: agents around the relay, and the relay drawn
 * as a glass card that holds one conversation. Each message travels as a
 * dot from its sender into the card, lands there once the relay has
 * checked it, and leaves for its peer; the last request stops in the card.
 *
 * The server renders the finished conversation, so the card reads in full
 * without script. With motion allowed, it holds that state for a moment,
 * clears, and replays while it is on screen.
 */

type NodeId = "ana" | "bob" | "anon" | "sam" | "kit" | "lee";

// The viewBox is 700 x 400 and the card covers roughly its middle 400
// units, so the agents sit in the two side bands, names facing outwards.
const HUB = { x: 350, y: 200 };

const NODES: Record<NodeId, { x: number; y: number; label: string; anchor: "start" | "end" }> = {
  ana: { x: 128, y: 92, label: "ana/concierge", anchor: "end" },
  sam: { x: 106, y: 204, label: "sam/calendar", anchor: "end" },
  kit: { x: 130, y: 314, label: "kit/books", anchor: "end" },
  bob: { x: 572, y: 86, label: "bob/venue-bot", anchor: "start" },
  lee: { x: 594, y: 200, label: "lee/tax", anchor: "start" },
  anon: { x: 572, y: 314, label: "anon/scraper", anchor: "start" },
};

type Step = {
  from: NodeId;
  to: NodeId;
  /** Stopped at the relay: the dot never reaches `to`. */
  refused?: boolean;
  /** The card row this step reveals. */
  row: Row;
};

type Row =
  | { kind: "msg"; who: string; text: string; side: "me" | "them" }
  | { kind: "tool" | "refused"; text: string; status: string };

const STEPS: Step[] = [
  {
    from: "ana",
    to: "bob",
    row: { kind: "msg", who: "ana/concierge", text: "Table for six on Friday?", side: "me" },
  },
  {
    from: "bob",
    to: "ana",
    row: { kind: "msg", who: "bob/venue-bot", text: "7:30 is free, by the window.", side: "them" },
  },
  {
    from: "ana",
    to: "bob",
    row: { kind: "msg", who: "ana/concierge", text: "Hold it, please.", side: "me" },
  },
  {
    from: "ana",
    to: "bob",
    row: { kind: "tool", text: "ana/concierge used hold_table", status: "allowed" },
  },
  {
    from: "anon",
    to: "bob",
    refused: true,
    row: { kind: "refused", text: "anon/scraper, no grant", status: "refused" },
  },
];

const LEG = 620; // ms for a dot to cross from a node to the relay
const GAP = 1500; // ms between messages
const HOLD = 3200; // ms the finished conversation stays up

const at = (p: { x: number; y: number }) => `translate(${p.x}px, ${p.y}px)`;

export default function RelayHero({ className = "" }: { className?: string }) {
  // Every row is shown until the replay starts.
  const [shown, setShown] = useState(STEPS.length);
  // The row whose message is on its way: it shows a typing bubble.
  const [typing, setTyping] = useState(-1);
  const rootRef = useRef<HTMLDivElement>(null);
  const dotRef = useRef<SVGGElement>(null);
  const okRef = useRef<SVGCircleElement>(null);
  const noRef = useRef<SVGCircleElement>(null);

  useEffect(() => {
    const root = rootRef.current;
    if (!root || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let run = 0;
    const timers = new Set<number>();
    const wait = (ms: number) =>
      new Promise<void>((done) => {
        const t = window.setTimeout(() => {
          timers.delete(t);
          done();
        }, ms);
        timers.add(t);
      });

    const pulse = (el: SVGCircleElement | null) =>
      el?.animate(
        [
          { transform: "scale(1)", opacity: 0.9 },
          { transform: "scale(2.1)", opacity: 0 },
        ],
        { duration: 700, easing: "cubic-bezier(.2,.7,.3,1)" },
      );

    const send = (step: Step) => {
      const dot = dotRef.current;
      if (!dot) return;
      const a = NODES[step.from];
      const b = NODES[step.to];
      const frames: Keyframe[] = step.refused
        ? [
            { transform: at(a), opacity: 0 },
            { opacity: 1, offset: 0.15 },
            { transform: at(HUB), opacity: 1, offset: 0.92 },
            { transform: `${at(HUB)} scale(0.2)`, opacity: 0 },
          ]
        : [
            { transform: at(a), opacity: 0 },
            { opacity: 1, offset: 0.08 },
            { transform: at(HUB), offset: 0.5 },
            { transform: at(b), opacity: 1, offset: 0.94 },
            { transform: at(b), opacity: 0 },
          ];
      dot.animate(frames, {
        duration: step.refused ? LEG * 1.1 : LEG * 2,
        easing: "cubic-bezier(.45,0,.25,1)",
        fill: "forwards",
      });
    };

    const play = async (id: number) => {
      await wait(2400);
      while (id === run) {
        setShown(0);
        await wait(900);
        for (let i = 0; i < STEPS.length; i++) {
          if (id !== run) return;
          const step = STEPS[i];
          setTyping(i);
          send(step);
          await wait(LEG);
          if (id !== run) return;
          // The relay has it: checked, it lands in the card (and, unless
          // refused, carries on to the peer while the card shows it).
          pulse(step.refused ? noRef.current : okRef.current);
          setTyping(-1);
          setShown(i + 1);
          await wait(GAP);
        }
        await wait(HOLD);
      }
    };

    const stop = () => {
      run++;
      for (const t of timers) window.clearTimeout(t);
      timers.clear();
    };

    const io = new IntersectionObserver(([e]) => {
      stop();
      if (e.isIntersecting) {
        void play(run);
      } else {
        setTyping(-1);
        setShown(STEPS.length);
      }
    });
    io.observe(root);
    return () => {
      io.disconnect();
      stop();
    };
  }, []);

  return (
    <div ref={rootRef} className={`${h.stage} ${className}`} data-ui>
      <svg className={h.net} viewBox="0 0 700 400" aria-hidden="true">
        <circle className={`${h.orbit} ${h.orbitA}`} cx={HUB.x} cy={HUB.y} r="150" />
        <circle className={`${h.orbit} ${h.orbitB}`} cx={HUB.x} cy={HUB.y} r="232" />
        {(Object.keys(NODES) as NodeId[]).map((id) => {
          const n = NODES[id];
          return (
            <line
              key={id}
              className={h.spoke}
              x1={n.x}
              y1={n.y}
              x2={HUB.x}
              y2={HUB.y}
            />
          );
        })}
        <circle className={h.hubRing} cx={HUB.x} cy={HUB.y} r="88" />
        <circle className={h.hub} cx={HUB.x} cy={HUB.y} r="40" />
        <circle ref={okRef} className={h.pulseOk} cx={HUB.x} cy={HUB.y} r="40" />
        <circle ref={noRef} className={h.pulseNo} cx={HUB.x} cy={HUB.y} r="40" />
        {(Object.keys(NODES) as NodeId[]).map((id, i) => {
          const n = NODES[id];
          const dx = n.anchor === "start" ? 16 : -16;
          return (
            <g key={id} className={h.node} style={{ animationDelay: `${i * -1.3}s` }}>
              <circle className={h.nodeDot} cx={n.x} cy={n.y} r="7" />
              <text className={h.nodeLabel} x={n.x + dx} y={n.y + 5} textAnchor={n.anchor}>
                {n.label}
              </text>
            </g>
          );
        })}
        <g ref={dotRef} className={h.packet}>
          <circle r="7" />
        </g>
      </svg>

      <div className={h.glass} role="group" aria-label="One conversation on the relay">
        <div className={h.glassHead}>
          <span className={h.live} aria-hidden="true" />
          <span className={h.glassTitle}>Dinner for six, Friday</span>
          <span className={h.glassMeta}>call on the relay</span>
        </div>
        <ol className={h.rows}>
          {STEPS.map(({ row }, i) => (
            <li
              key={i}
              className={`${h.row} ${rowClass(row)} ${i < shown ? h.in : ""} ${
                i === typing ? h.typing : ""
              }`}
            >
              <span className={h.dots} aria-hidden="true">
                <i />
                <i />
                <i />
              </span>
              {row.kind === "msg" ? (
                <>
                  <span className={h.who}>{row.who}</span>
                  <span>{row.text}</span>
                </>
              ) : (
                <>
                  <span>{row.text}</span>
                  <b>{row.status}</b>
                </>
              )}
            </li>
          ))}
        </ol>
      </div>
    </div>
  );
}

function rowClass(row: Row) {
  if (row.kind !== "msg") return row.kind === "tool" ? h.tool : h.refused;
  return row.side === "me" ? h.me : h.them;
}
