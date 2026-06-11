// One-shot: rasterize public/brand/mark.svg to the 180×180 PNG Apple
// touch icon. Re-run with `node scripts/gen-touch-icon.mjs` if the
// mark changes. Uses the Playwright chromium already in devDeps.
import { chromium } from "@playwright/test";
import { readFileSync } from "node:fs";

const svg = readFileSync("public/brand/mark.svg", "utf8");
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 180, height: 180 } });
// Solid paper-cream background — iOS composites touch icons onto black
// when the PNG has alpha, which looks broken against the home screen.
await page.setContent(`<!doctype html><style>html,body{margin:0;padding:0;background:#faf6ef}svg{width:180px;height:180px;display:block}</style>${svg}`);
await page.screenshot({ path: "public/brand/apple-touch-icon.png" });
await browser.close();
console.log("written public/brand/apple-touch-icon.png");
