// The relay's three answers: one stranger refused, one friend allowed, one bigger ask held for the owner.
// 20 s (600 frames at 30 fps), 1080x1080, silent. Every name, tool, verdict and number is from the landing page
// (frontend/src/app/(site)/page.tsx); the look is brand/brand.css, written from the site's tokens.
import { AbsoluteFill, Animate, Composition, Img, Sequence, interpolate, registerRoot, staticFile, useCurrentFrame, useVideoConfig } from "raea";
import { css } from "./styles.gen";

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;
const SCENE = 165; // 5.5 s each
const END = 105; // 3.5 s

type Tone = "no" | "yes" | "wait";
type Case = {
  line: string;
  from: string;
  tool: string;
  checks: [string, string, boolean][]; // label, value, passed
  verdict: string;
  after: string;
  tone: Tone;
};

const CASES: Case[] = [
  {
    line: "A stranger's agent asks\nyours to charge a card.",
    from: "anon/scraper",
    tool: "charge_deposit",
    checks: [["Friendship", "none", false], ["Grant for charge_deposit", "none", false]],
    verdict: "Refused",
    after: "It never reached bob/venue-bot.",
    tone: "no",
  },
  {
    line: "A friend's agent asks\nto hold a table.",
    from: "ana/concierge",
    tool: "hold_table",
    checks: [["Friendship", "yes", true], ["Grant for hold_table", "active", true], ["Message", "read first", true]],
    verdict: "Allowed",
    after: "38 ms",
    tone: "yes",
  },
  {
    line: "The same friend asks\nto charge the deposit.",
    from: "ana/concierge",
    tool: "charge_deposit",
    checks: [["Friendship", "yes", true], ["Grant for charge_deposit", "asks the owner", true]],
    verdict: "Waiting for bob's owner",
    after: "up to 7 days",
    tone: "wait",
  },
];

const BAR: Record<Tone, { bg: string; fg: string }> = {
  no: { bg: "var(--brand-accent)", fg: "var(--brand-on-coral)" },
  yes: { bg: "var(--brand-lime)", fg: "var(--brand-text)" },
  wait: { bg: "var(--brand-butter)", fg: "var(--brand-text)" },
};

// frames inside a scene
const T = { line: 0, request: 14, check: 44, each: 16, verdictGap: 14, out: SCENE - 8 };

/** One request through the relay: the line above, then the card fills in and the verdict lands. */
function Scene({ c }: { c: Case }) {
  const f = useCurrentFrame();
  const { width } = useVideoConfig();
  const u = width / 1080;
  const verdictAt = T.check + c.checks.length * T.each + T.verdictGap;
  // the waiting bar breathes; the others hold
  const breathe = c.tone === "wait" ? 0.82 + 0.18 * Math.cos(((f - verdictAt) / 30) * Math.PI * 2 * 0.8) : 1;
  const sweep = interpolate(f, [verdictAt, verdictAt + 9], [0, 100], clamp);
  return (
    <AbsoluteFill>
      <Animate enter="rise" at={T.line} exit="fade" exitAt={T.out} style={{
        position: "absolute", left: 70 * u, top: 96 * u, width: 940 * u, textAlign: "left",
        fontFamily: "var(--brand-font-heading)", fontWeight: 400, fontSize: 88 * u, lineHeight: 1.0,
        letterSpacing: "-0.05em", color: "var(--brand-text)", whiteSpace: "pre",
      }}>{c.line}</Animate>

      <div style={{
        position: "absolute", left: 70 * u, top: 380 * u, width: 940 * u, height: 570 * u, boxSizing: "border-box",
        padding: `${40 * u}px ${44 * u}px`, borderRadius: 28 * u, backgroundColor: "var(--brand-paper-soft)",
        display: "flex", flexDirection: "column", fontFamily: "var(--brand-font-mono)", color: "var(--brand-text)",
      }}>
        <Animate enter="slide" at={T.request} exit="fade" exitAt={T.out} style={{ display: "flex", flexDirection: "column", gap: 10 * u }}>
          <div style={{ fontSize: 40 * u, lineHeight: 1.1, fontWeight: 500, whiteSpace: "nowrap" }}>
            {c.from} <span style={{ color: "var(--brand-ink-muted)" }}>→</span> bob/venue-bot
          </div>
          <div style={{ fontSize: 40 * u, lineHeight: 1.1, fontWeight: 400, color: "var(--brand-ink-soft)", whiteSpace: "nowrap" }}>{c.tool}</div>
        </Animate>

        <div style={{ marginTop: 36 * u, display: "flex", flexDirection: "column", gap: 14 * u }}>
          {c.checks.map(([label, value, ok], i) => (
            <Animate key={label} enter="rise" at={T.check + i * T.each} exit="fade" exitAt={T.out} style={{
              display: "flex", justifyContent: "space-between", alignItems: "baseline", fontFamily: "var(--brand-font)",
              fontSize: 40 * u, lineHeight: 1.15, fontWeight: 400, whiteSpace: "nowrap",
            }}>
              <span style={{ color: "var(--brand-ink-soft)" }}>{label}</span>
              <span style={{ fontWeight: 500, color: ok ? "var(--brand-text)" : "var(--brand-accent)" }}>{value}</span>
            </Animate>
          ))}
        </div>

        <div style={{
          position: "absolute", left: 44 * u, right: 44 * u, bottom: 40 * u, height: 104 * u, borderRadius: 18 * u,
          overflow: "hidden", opacity: interpolate(f, [T.out, T.out + 6], [1, 0], clamp),
        }}>
          <div style={{
            position: "absolute", left: 0, top: 0, bottom: 0, width: `${sweep}%`, backgroundColor: BAR[c.tone].bg, opacity: breathe,
          }} />
          <Animate enter="fade" at={verdictAt + 4} style={{
            position: "absolute", left: 30 * u, right: 30 * u, top: 0, bottom: 0, display: "flex", alignItems: "center",
            justifyContent: "space-between", fontFamily: "var(--brand-font)", color: BAR[c.tone].fg, whiteSpace: "nowrap",
          }}>
            <span style={{ fontSize: 46 * u, fontWeight: 500, letterSpacing: "-0.03em" }}>{c.verdict}</span>
            <span style={{ fontSize: 38 * u, fontWeight: 400 }}>{c.after}</span>
          </Animate>
        </div>
      </div>
    </AbsoluteFill>
  );
}

/** The ask: the headline, the wordmark and the address on ink. */
function EndCard() {
  const { width } = useVideoConfig();
  const u = width / 1080;
  return (
    <AbsoluteFill style={{ backgroundColor: "var(--brand-text)" }}>
      <Animate enter="rise" at={4} style={{
        position: "absolute", left: 70 * u, top: 330 * u, width: 940 * u, textAlign: "left",
        fontFamily: "var(--brand-font-heading)", fontWeight: 400, fontSize: 136 * u, lineHeight: 0.98,
        letterSpacing: "-0.05em", color: "var(--brand-on-ink)",
      }}>Agents talk,<br />you set the rules.</Animate>
      <Animate enter="fade" at={22} style={{
        position: "absolute", left: 70 * u, right: 70 * u, top: 840 * u, display: "flex", alignItems: "center",
        justifyContent: "space-between",
      }}>
        <Img src={staticFile("brand/chakramcp.com-icon.png")} style={{ width: 96 * u, height: 96 * u, borderRadius: 22 * u, display: "block" }} />
        <span style={{ fontFamily: "var(--brand-font-mono)", fontSize: 44 * u, fontWeight: 500, color: "var(--brand-on-ink)" }}>chakramcp.com</span>
      </Animate>
    </AbsoluteFill>
  );
}

export default function RelayCheck() {
  return (
    <AbsoluteFill style={{ backgroundColor: "var(--brand-background)" }}>
      <style>{css}</style>
      {CASES.map((c, i) => (
        <Sequence key={i} from={i * SCENE} durationInFrames={SCENE} name={c.verdict}><Scene c={c} /></Sequence>
      ))}
      <Sequence from={CASES.length * SCENE} durationInFrames={END} name="ask"><EndCard /></Sequence>
    </AbsoluteFill>
  );
}

registerRoot(() => <Composition id="relay-check" component={RelayCheck} width={1080} height={1080} fps={30} durationInFrames={CASES.length * SCENE + END} />);
