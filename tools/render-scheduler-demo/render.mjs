// Offline renderer: produces a 2-terminal MP4 + GIF showing the
// scheduler-demo flow (Alice's inbox.serve loop + Bob's invoke).
//
// Usage:
//   pnpm install
//   pnpm render
//
// Outputs to ../../examples/scheduler-demo/{scheduler-demo.mp4,scheduler-demo.gif}
// and a copy in ../../frontend/public/assets/ for the marketing site.

import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
import {
  existsSync, mkdirSync, readdirSync, rmSync, unlinkSync, copyFileSync,
} from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const FRAMES_DIR = resolve(here, "frames");
const OUT_DIR = resolve(here, "output");
const DEMO_DIR = resolve(here, "..", "..", "examples", "scheduler-demo");
const PUBLIC_ASSETS = resolve(here, "..", "..", "frontend", "public", "assets");

const FPS = 30;
const LOOP_MS = 14_000;
const TOTAL_FRAMES = (LOOP_MS / 1000) * FPS;
const VIEWPORT = { width: 1400, height: 720 };

const MP4_OUT = resolve(OUT_DIR, "scheduler-demo.mp4");
const GIF_OUT = resolve(OUT_DIR, "scheduler-demo.gif");
const PALETTE = resolve(OUT_DIR, "palette.png");

// Both transcripts come from the live smoke-test run on 2026-04-28.
// Each entry: { tStart: ms when this line begins typing, line: string }.
// Lines on Alice are typed left-pane; lines on Bob right-pane. Times are
// hand-tuned for legibility.
const TIMELINE = {
  alice: [
    { t: 200,  line: "$ python alice_scheduler.py" },
    { t: 1200, line: "signed in as demo-alice@example.com" },
    { t: 1500, line: "agent  : 019dd0f8-9b36-7fa2-9f1b-7450c479c2b8" },
    { t: 1800, line: "" },
    { t: 1900, line: "Listening for invocations… (ctrl-c to stop)" },
    { t: 2200, line: "" },
    // After Bob fires at ~6500ms, Alice's handler picks up and logs:
    { t: 6700, line: "  ← propose_slots({'duration_min': 30, 'within_days': 7})" },
    { t: 7300, line: "    grant 019dd0f8-9ba1…  visibility=network" },
    { t: 7700, line: "    friendship 019dd0f8-9b85…  initial: 'Want to schedule a meeting?'" },
    { t: 8100, line: "  → returning 4 slots" },
  ],
  bob: [
    { t: 5000, line: "$ python bob_caller.py" },
    { t: 6000, line: "signed in as demo-bob@example.com" },
    { t: 6300, line: "calling alice-scheduler.propose_slots through grant 019dd0f8-9ba1…" },
    { t: 6600, line: "" },
    { t: 8400, line: "  status     : succeeded" },
    { t: 8700, line: "  elapsed_ms : 23" },
    { t: 9000, line: "  slots      : 4" },
    { t: 9300, line: "    • 2026-04-28T09:00:00+00:00" },
    { t: 9500, line: "    • 2026-04-28T10:00:00+00:00" },
    { t: 9700, line: "    • 2026-04-28T13:00:00+00:00" },
    { t: 9900, line: "    • 2026-05-03T09:00:00+00:00" },
  ],
};

function pageHTML() {
  // Self-contained: no external CSS, no fonts beyond system mono.
  // Uses the brand palette (cream/coral/ink). Each terminal is a
  // .term with .pane > .line[data-t-start][data-t-end] children.
  // The render script drives `data-now` on body to reveal lines whose
  // tStart <= now, character-by-character within their typing window.
  return `<!doctype html>
<html><head><meta charset="utf-8"><style>
  :root {
    --paper: oklch(96% 0.014 72);
    --paper-warm: oklch(94% 0.024 70);
    --ink: oklch(28% 0.04 60);
    --ink-soft: oklch(45% 0.04 60);
    --line: oklch(86% 0.02 70);
    --coral: oklch(63% 0.19 28);
    --term-bg: oklch(20% 0.025 60);
    --term-fg: oklch(94% 0.014 72);
    --term-dim: oklch(70% 0.02 70);
    --term-accent: oklch(78% 0.16 60);
    --term-coral: oklch(72% 0.16 28);
    --term-lime: oklch(85% 0.13 130);
  }
  * { box-sizing: border-box; }
  html, body { margin: 0; padding: 0; height: 100%; }
  body {
    background: var(--paper);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    color: var(--ink);
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: 14px;
    padding: 22px 28px;
    overflow: hidden;
  }
  .heading {
    display: flex; align-items: center; gap: 14px;
  }
  .heading .dot {
    width: 18px; height: 18px; border-radius: 999px;
    background: var(--coral);
    box-shadow: 0 0 0 5px color-mix(in oklab, var(--coral) 18%, transparent);
  }
  .heading h1 {
    margin: 0; font-size: 18px; font-weight: 700; letter-spacing: 0.01em;
  }
  .heading .sub {
    margin-left: auto; color: var(--ink-soft); font-size: 13px;
  }
  .stage {
    display: grid; grid-template-columns: 1fr 1fr; gap: 16px;
    min-height: 0;
  }
  .term {
    display: grid; grid-template-rows: auto 1fr;
    background: var(--term-bg);
    border-radius: 12px;
    overflow: hidden;
    box-shadow: 0 6px 20px rgba(0,0,0,0.10), inset 0 0 0 1px rgba(255,255,255,0.04);
  }
  .term-chrome {
    display: flex; align-items: center; gap: 8px;
    padding: 10px 14px;
    background: linear-gradient(180deg, oklch(28% 0.03 60), oklch(22% 0.025 60));
    border-bottom: 1px solid rgba(255,255,255,0.06);
  }
  .traffic { display: flex; gap: 6px; }
  .traffic span {
    width: 11px; height: 11px; border-radius: 999px;
    background: oklch(68% 0.18 25);
  }
  .traffic span:nth-child(2) { background: oklch(82% 0.16 80); }
  .traffic span:nth-child(3) { background: oklch(70% 0.18 145); }
  .term-title {
    color: var(--term-dim); font: 12px ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    margin-left: 6px;
  }
  .pane {
    padding: 14px 16px;
    color: var(--term-fg);
    font: 13.5px/1.55 ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    overflow: hidden;
    white-space: pre;
  }
  .line { display: block; min-height: 1.55em; }
  .line.hidden { visibility: hidden; }
  .line .tag-prompt { color: var(--term-coral); font-weight: 700; }
  .line .tag-arrow { color: var(--term-lime); }
  .cursor {
    display: inline-block; width: 7px; height: 1.05em;
    background: var(--term-fg); margin-left: 1px; vertical-align: -2px;
    animation: blink 1s steps(1) infinite;
  }
  @keyframes blink { 50% { opacity: 0; } }
  .footer {
    display: flex; align-items: center; gap: 10px;
    color: var(--ink-soft); font-size: 12.5px;
    padding-top: 4px;
  }
  .footer .pill {
    padding: 3px 9px; border: 1px solid var(--line);
    border-radius: 999px; background: var(--paper-warm); color: var(--ink);
    font-weight: 600;
  }
</style></head>
<body data-now="0">
  <div class="heading">
    <span class="dot" aria-hidden="true"></span>
    <h1>scheduler-demo - two agents through one relay</h1>
    <span class="sub">examples/scheduler-demo</span>
  </div>
  <div class="stage">
    <section class="term" id="term-alice">
      <div class="term-chrome">
        <div class="traffic"><span></span><span></span><span></span></div>
        <div class="term-title">alice@scheduler  -  inbox.serve()</div>
      </div>
      <div class="pane" data-pane="alice"></div>
    </section>
    <section class="term" id="term-bob">
      <div class="term-chrome">
        <div class="traffic"><span></span><span></span><span></span></div>
        <div class="term-title">bob@caller  -  invoke_and_wait()</div>
      </div>
      <div class="pane" data-pane="bob"></div>
    </section>
  </div>
  <div class="footer">
    <span class="pill">friendship</span><span>accepted</span>
    <span class="pill">grant</span><span>active &middot; propose_slots</span>
    <span class="pill">trust context</span><span>relay-bundled, no extra round-trip</span>
  </div>

<script>
  const TIMELINE = ${JSON.stringify(TIMELINE)};
  const TYPE_MS = 320; // each line types over this many ms
  const PANE_HEIGHT_PX = 460;

  function buildPane(name) {
    const pane = document.querySelector('[data-pane="' + name + '"]');
    pane.innerHTML = "";
    for (const item of TIMELINE[name]) {
      const span = document.createElement("span");
      span.className = "line hidden";
      span.dataset.tStart = String(item.t);
      span.dataset.tEnd = String(item.t + TYPE_MS);
      span.dataset.full = item.line;
      span.textContent = "";
      pane.appendChild(span);
    }
  }
  buildPane("alice");
  buildPane("bob");

  function frameStyle(line) {
    const s = line.textContent;
    // Highlight prompts and arrows for visual emphasis
    if (s.startsWith("$ ")) {
      line.innerHTML = '<span class="tag-prompt">$</span> ' + escapeHtml(s.slice(2));
    } else if (s.includes("←")) {
      line.innerHTML = '<span class="tag-arrow">' + escapeHtml(s.slice(0, 4)) + '</span>' + escapeHtml(s.slice(4));
    } else if (s.includes("→")) {
      line.innerHTML = '<span class="tag-arrow">' + escapeHtml(s.slice(0, 4)) + '</span>' + escapeHtml(s.slice(4));
    } else {
      line.textContent = s;
    }
  }
  function escapeHtml(t) {
    return t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  function render(now) {
    document.body.dataset.now = String(now);
    for (const pane of ["alice", "bob"]) {
      const lines = document.querySelectorAll('[data-pane="' + pane + '"] .line');
      let visibleLast = -1;
      lines.forEach((line, idx) => {
        const t0 = +line.dataset.tStart;
        const t1 = +line.dataset.tEnd;
        const full = line.dataset.full;
        if (now < t0) {
          line.classList.add("hidden");
          line.textContent = "";
        } else {
          line.classList.remove("hidden");
          if (now >= t1 || full.length === 0) {
            line.textContent = full;
          } else {
            const p = (now - t0) / (t1 - t0);
            const n = Math.floor(full.length * p);
            line.textContent = full.slice(0, n);
          }
          frameStyle(line);
          visibleLast = idx;
        }
      });
      // Cursor on the last visible line
      const cursor = document.querySelectorAll('[data-pane="' + pane + '"] .cursor');
      cursor.forEach((c) => c.remove());
      if (visibleLast >= 0) {
        const cur = document.createElement("span");
        cur.className = "cursor";
        lines[visibleLast].appendChild(cur);
      }
    }
  }

  // Expose a renderer the headless driver can call.
  window.__renderAt = (ms) => render(ms);
  // Initial frame.
  render(0);
</script>
</body></html>`;
}

async function main() {
  // Wipe frames.
  if (existsSync(FRAMES_DIR)) {
    for (const f of readdirSync(FRAMES_DIR)) unlinkSync(resolve(FRAMES_DIR, f));
  } else {
    mkdirSync(FRAMES_DIR, { recursive: true });
  }
  mkdirSync(OUT_DIR, { recursive: true });

  console.log("[render] launching chromium");
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  const page = await ctx.newPage();

  await page.setContent(pageHTML(), { waitUntil: "load" });
  await page.evaluate(() => document.fonts.ready);

  console.log(`[render] capturing ${TOTAL_FRAMES} frames at ${FPS}fps`);
  for (let i = 0; i < TOTAL_FRAMES; i++) {
    const t = (i / TOTAL_FRAMES) * LOOP_MS;
    await page.evaluate((t) => window.__renderAt(t), t);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r(null))));
    const path = resolve(FRAMES_DIR, `frame_${String(i).padStart(4, "0")}.png`);
    await page.screenshot({ path, fullPage: false });
    if (i % 30 === 0) process.stdout.write(`  ${i}/${TOTAL_FRAMES}\r`);
  }
  console.log(`\n[render] frames captured`);

  await browser.close();

  console.log(`[render] encoding MP4`);
  run("ffmpeg", [
    "-y",
    "-framerate", String(FPS),
    "-i", resolve(FRAMES_DIR, "frame_%04d.png"),
    "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
    "-c:v", "libx264", "-preset", "slow", "-crf", "20",
    "-movflags", "+faststart", "-an",
    MP4_OUT,
  ]);

  console.log(`[render] generating GIF (palette pass)`);
  run("ffmpeg", [
    "-y",
    "-framerate", String(FPS),
    "-i", resolve(FRAMES_DIR, "frame_%04d.png"),
    "-vf", "fps=20,scale=900:-1:flags=lanczos,palettegen=stats_mode=full",
    PALETTE,
  ]);
  run("ffmpeg", [
    "-y",
    "-framerate", String(FPS),
    "-i", resolve(FRAMES_DIR, "frame_%04d.png"),
    "-i", PALETTE,
    "-lavfi", "fps=20,scale=900:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle",
    GIF_OUT,
  ]);
  rmSync(PALETTE, { force: true });

  // Distribute the artifacts.
  mkdirSync(DEMO_DIR, { recursive: true });
  copyFileSync(MP4_OUT, resolve(DEMO_DIR, "scheduler-demo.mp4"));
  copyFileSync(GIF_OUT, resolve(DEMO_DIR, "scheduler-demo.gif"));
  if (existsSync(PUBLIC_ASSETS)) {
    copyFileSync(MP4_OUT, resolve(PUBLIC_ASSETS, "scheduler-demo.mp4"));
    copyFileSync(GIF_OUT, resolve(PUBLIC_ASSETS, "scheduler-demo.gif"));
  }

  console.log(`[render] done`);
  console.log(`  ${MP4_OUT}`);
  console.log(`  ${GIF_OUT}`);
  console.log(`  copied to examples/scheduler-demo/ and frontend/public/assets/`);
}

function run(bin, args) {
  execFileSync(bin, args, { stdio: "inherit" });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
