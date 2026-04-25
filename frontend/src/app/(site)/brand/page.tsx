import type { Metadata } from "next";
import Image from "next/image";
import styles from "./brand.module.css";

export const metadata: Metadata = {
  title: "Brand \u2014 ChakraMCP",
  description:
    "Brand identity, voice, colors, type, logo, and a downloadable Claude Code skill for building on the ChakraMCP brand.",
  robots: { index: false, follow: false },
};

const swatches = [
  { name: "paper", value: "var(--paper)", note: "Default page background \u2014 cream." },
  { name: "paper-warm", value: "var(--paper-warm)", note: "Warmer cream \u2014 cards, hover states." },
  { name: "ink", value: "var(--ink)", note: "Primary text \u2014 dark brown-black." },
  { name: "accent-coral", value: "var(--accent-coral)", note: "Primary accent \u2014 terra cotta." },
  { name: "accent-lime", value: "var(--accent-lime)", note: "Unexpected lime \u2014 secondary." },
  { name: "accent-butter", value: "var(--accent-butter)", note: "Butter yellow \u2014 highlights." },
];

const voiceRules = [
  {
    do: "Short declarative sentences. Mischievous, not corporate.",
    dont: "No \u201cempower\u201d, no \u201cleverage\u201d, no \u201crevolutionary.\u201d",
  },
  {
    do: "Metaphors rooted in physical venues \u2014 \u201cthe relay is the bouncer.\u201d",
    dont: "No abstract AI-speak. No \u201cunlock synergies.\u201d",
  },
  {
    do: "Explain the paperwork. Friendship, grant, consent, audit.",
    dont: "No hand-waving about \u201cAI trust\u201d or \u201cautonomous coordination.\u201d",
  },
  {
    do: "Editorial and zine-like. A good sentence is better than a feature list.",
    dont: "No dashboards. No dark mode. No neon.",
  },
];

const downloads = [
  {
    name: "Brand kit (zip)",
    file: "chakramcp-brand-kit.zip",
    path: "/assets/chakramcp-brand-kit.zip",
    description:
      "Logo mark + wordmark, design tokens (colors, type, spacing), and the Claude Code design skill. Drop the skill in your .claude/skills folder and Claude can generate ChakraMCP-branded UI.",
  },
  {
    name: "Composite mark (SVG)",
    file: "mark-composite.svg",
    path: "/brand/mark-composite.svg",
    description:
      "v3 composite — floating hub with seven satellites + pulsing chakra + relay arc. Hero-scale lockup. Use on big surfaces (decks, OG, posters).",
  },
  {
    name: "Simple mark (SVG)",
    file: "mark.svg",
    path: "/brand/mark.svg",
    description: "The coral dot, alone. Use at 16px and up — favicons, inline mentions.",
  },
  {
    name: "Wordmark (SVG)",
    file: "wordmark.svg",
    path: "/brand/wordmark.svg",
    description: "ChakraMCP wordmark. Archivo Expanded, tracking tuned for lockup.",
  },
  {
    name: "Design tokens (CSS)",
    file: "colors_and_type.css",
    path: "/assets/colors_and_type.css",
    description: "The whole token system: colors, type, spacing, radii, shadows, motion.",
  },
  {
    name: "Claude Code skill (MD)",
    file: "chakramcp-design-skill.md",
    path: "/assets/chakramcp-design-skill.md",
    description:
      "The skill frontmatter that teaches Claude Code to apply the ChakraMCP brand when generating artifacts.",
  },
  {
    name: "Coffee-loop (MP4)",
    file: "coffee-loop.mp4",
    path: "/assets/coffee-loop.mp4",
    description:
      "The dispatch-log animation as a 12-second MP4. 1200×720, H.264, muted, loop-friendly. Drop it into a social post or a deck.",
  },
  {
    name: "Coffee-loop (GIF)",
    file: "coffee-loop.gif",
    path: "/assets/coffee-loop.gif",
    description:
      "Same animation as a GIF for places that don\u2019t play MP4 inline. 800×480, 15fps.",
  },
];

export default function BrandPage() {
  return (
    <>
      <section className="hero-block hero-block--concept">
        <div className="hero-copy reveal">
          <div className="eyebrow">Brand page</div>
          <h1>Editorial, warm, rogue-zine. Not a dashboard.</h1>
          <p className="lead">
            ChakraMCP has a voice. Cream paper, ink text, coral and lime accents, Archivo Expanded
            headlines, Afacad body. Short sentences. Physical-venue metaphors. No blue. No neon. No
            emoji.
          </p>
          <div className="hero-actions">
            <a className="pill-link pill-link--primary" href="/assets/chakramcp-brand-kit.zip" download>
              Download brand kit
            </a>
            <a className="pill-link" href="/assets/chakramcp-design-skill.md" download>
              Get the Claude Code skill
            </a>
          </div>
        </div>
        <aside className="hero-board reveal">
          <div className="note-badge">The coral dot</div>
          <div className={styles.logoStage}>
            <Image
              src="/brand/mark.svg"
              alt="ChakraMCP mark"
              width={96}
              height={96}
              priority
            />
            <Image
              src="/brand/wordmark.svg"
              alt="ChakraMCP wordmark"
              width={240}
              height={48}
              priority
            />
          </div>
          <p className="hero-board-copy">
            The mark is a single coral dot with a soft halo. The wordmark sits next to it in
            Archivo Expanded. Never stretch, never recolor.
          </p>
        </aside>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">01</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Color</div>
            <h2>Six load-bearing tones.</h2>
          </div>
          <div className={styles.swatchGrid}>
            {swatches.map((s) => (
              <div key={s.name} className={styles.swatchCard}>
                <div className={styles.swatchSwatch} style={{ background: s.value }} />
                <div className={styles.swatchMeta}>
                  <code>{s.name}</code>
                  <p>{s.note}</p>
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">02</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Type</div>
            <h2>Archivo Expanded for display. Afacad for body.</h2>
          </div>
          <div className={styles.typeSpecimen}>
            <div className={styles.typeRow}>
              <span className="eyebrow">Display \u2014 Archivo Expanded 800</span>
              <span className={styles.typeDisplay}>The relay is the bouncer.</span>
            </div>
            <div className={styles.typeRow}>
              <span className="eyebrow">Body \u2014 Afacad 400</span>
              <p className={styles.typeBody}>
                Every remote call passes through the relay. Friendship state, grant scope, consent
                records, quotas, and audit policy all get checked before the target agent ever
                sees the request. Discovery is public. Access is negotiated.
              </p>
            </div>
            <div className={styles.typeRow}>
              <span className="eyebrow">Mono \u2014 system mono</span>
              <code className={styles.typeMono}>POST /v1/proposals \u2014 200 OK</code>
            </div>
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">03</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Voice</div>
            <h2>Do. Don\u2019t.</h2>
          </div>
          <div className={styles.voiceGrid}>
            {voiceRules.map((v, i) => (
              <div key={i} className={styles.voicePair}>
                <div className={styles.voiceDo}>
                  <div className="eyebrow">Do</div>
                  <p>{v.do}</p>
                </div>
                <div className={styles.voiceDont}>
                  <div className="eyebrow">Don\u2019t</div>
                  <p>{v.dont}</p>
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">04</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Downloads</div>
            <h2>Assets and a skill you can run in Claude Code.</h2>
            <p>
              The brand kit is a zip. The skill is a single markdown file with frontmatter \u2014
              drop it in your <code>.claude/skills/</code> folder and Claude will know how to build
              on the ChakraMCP brand.
            </p>
          </div>
          <ul className={styles.downloadList}>
            {downloads.map((d) => (
              <li key={d.file} className={styles.downloadItem}>
                <div>
                  <h3>{d.name}</h3>
                  <p>{d.description}</p>
                  <code>{d.file}</code>
                </div>
                <a className="pill-link" href={d.path} download>
                  Download
                </a>
              </li>
            ))}
          </ul>
        </div>
      </section>
    </>
  );
}
