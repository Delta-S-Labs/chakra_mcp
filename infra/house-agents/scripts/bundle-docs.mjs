// Packs the docs the Guide answers from into dist/docs.json. Run from
// infra/house-agents (npm run build does). The repo root is two levels up,
// or DOCS_ROOT.
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(process.env.DOCS_ROOT ?? join(here, "../../.."));
const MAX_BYTES = 130_000;

const selfHosting = readdirSync(join(root, "docs/self-hosting"))
  .filter((f) => f.endsWith(".md"))
  .sort()
  .map((f) => `docs/self-hosting/${f}`);

const paths = [
  "README.md",
  "docs/INSTALL.md",
  ...selfHosting,
  "frontend/public/llms.txt",
  ".claude/skills/chakramcp-agent/SKILL.md",
];

const docs = paths.map((path) => ({ path, text: readFileSync(join(root, path), "utf8") }));
const bytes = docs.reduce((n, d) => n + Buffer.byteLength(d.text), 0);
if (bytes > MAX_BYTES) {
  console.error(`docs bundle is ${bytes} bytes, over the ${MAX_BYTES} limit: trim the list in scripts/bundle-docs.mjs`);
  process.exit(1);
}
mkdirSync(join(here, "../dist"), { recursive: true });
writeFileSync(join(here, "../dist/docs.json"), JSON.stringify(docs));
console.log(`bundled ${docs.length} docs, ${bytes} bytes`);
