// Offline renderer: captures frames of the /render/coffee-loop page at
// precise timeline points and composes them into an MP4 and a GIF via
// ffmpeg.
//
// Usage:
//   1. `cd frontend && pnpm dev` in one terminal
//   2. `cd tools/render-coffee-loop && pnpm install && pnpm render` in another
//
// The script drives the CSS animation timeline deterministically by
// setting `animation.currentTime` on every active CSSAnimation before
// each screenshot — so the output is frame-perfect regardless of
// wall-clock drift.

import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync, unlinkSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const FRAMES_DIR = resolve(here, "frames");
const OUTPUT_DIR = resolve(here, "output");
const PUBLIC_ASSETS = resolve(here, "..", "..", "frontend", "public", "assets");

const URL = process.env.RENDER_URL ?? "http://localhost:3000/render/coffee-loop";
const LOOP_MS = 12_000;
const FPS = 30;
const TOTAL_FRAMES = (LOOP_MS / 1000) * FPS; // 360
const VIEWPORT = { width: 1200, height: 720 };

// Size the MP4 to viewport; GIF is downscaled for file size.
const MP4_OUT = resolve(OUTPUT_DIR, "coffee-loop.mp4");
const GIF_OUT = resolve(OUTPUT_DIR, "coffee-loop.gif");
const GIF_PALETTE = resolve(OUTPUT_DIR, "palette.png");

async function main() {
  await waitForServer(URL);

  // Wipe the frames folder so old runs don't contaminate this one.
  if (existsSync(FRAMES_DIR)) {
    for (const f of readdirSync(FRAMES_DIR)) unlinkSync(resolve(FRAMES_DIR, f));
  } else {
    mkdirSync(FRAMES_DIR, { recursive: true });
  }
  mkdirSync(OUTPUT_DIR, { recursive: true });

  console.log(`[render] launching headless chromium`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 1 });

  console.log(`[render] loading ${URL}`);
  await page.goto(URL, { waitUntil: "networkidle" });

  // Wait for fonts + first paint so dispatch-log lines are laid out.
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);

  // Pause all animations globally; we'll drive currentTime by hand.
  await page.evaluate(() => {
    document.getAnimations().forEach((a) => a.pause());
  });

  console.log(`[render] capturing ${TOTAL_FRAMES} frames at ${FPS}fps`);
  for (let i = 0; i < TOTAL_FRAMES; i++) {
    const t = (i / TOTAL_FRAMES) * LOOP_MS;
    await page.evaluate((t) => {
      for (const a of document.getAnimations()) {
        // For infinite animations, clamp to a single cycle.
        const dur = a.effect && a.effect.getComputedTiming().duration;
        const d = typeof dur === "number" ? dur : 12000;
        a.currentTime = t % d;
      }
    }, t);
    // Allow the next frame to paint the new state.
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r(null))));
    const path = resolve(FRAMES_DIR, `frame_${String(i).padStart(4, "0")}.png`);
    await page.screenshot({ path, fullPage: false });
    if (i % 30 === 0) process.stdout.write(`  ${i}/${TOTAL_FRAMES}\r`);
  }
  console.log(`\n[render] frames captured`);

  await browser.close();

  // Compose MP4 with ffmpeg.
  console.log(`[render] encoding MP4 (H.264, yuv420p, ${FPS}fps)`);
  run("ffmpeg", [
    "-y",
    "-framerate", String(FPS),
    "-i", resolve(FRAMES_DIR, "frame_%04d.png"),
    "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
    "-c:v", "libx264",
    "-preset", "slow",
    "-crf", "20",
    "-movflags", "+faststart",
    "-an",
    MP4_OUT,
  ]);

  // Generate palette + GIF (higher quality than single-pass).
  console.log(`[render] generating GIF palette`);
  run("ffmpeg", [
    "-y",
    "-i", MP4_OUT,
    "-vf", "fps=15,scale=800:-1:flags=lanczos,palettegen=max_colors=96",
    GIF_PALETTE,
  ]);

  console.log(`[render] encoding GIF`);
  run("ffmpeg", [
    "-y",
    "-i", MP4_OUT,
    "-i", GIF_PALETTE,
    "-lavfi", "fps=15,scale=800:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=5",
    GIF_OUT,
  ]);

  // Copy to frontend public so Netlify ships them.
  mkdirSync(PUBLIC_ASSETS, { recursive: true });
  const mp4Public = resolve(PUBLIC_ASSETS, "coffee-loop.mp4");
  const gifPublic = resolve(PUBLIC_ASSETS, "coffee-loop.gif");
  run("cp", [MP4_OUT, mp4Public]);
  run("cp", [GIF_OUT, gifPublic]);

  // Tidy: remove the palette + frames. Keep output MP4/GIF around locally.
  if (existsSync(GIF_PALETTE)) unlinkSync(GIF_PALETTE);
  rmSync(FRAMES_DIR, { recursive: true, force: true });

  console.log(`[render] done`);
  console.log(`  mp4: ${MP4_OUT}  →  ${mp4Public}`);
  console.log(`  gif: ${GIF_OUT}  →  ${gifPublic}`);
}

function run(cmd, args) {
  execFileSync(cmd, args, { stdio: "inherit" });
}

async function waitForServer(url, timeoutMs = 30_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // still booting
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`dev server at ${url} did not respond within ${timeoutMs}ms`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
