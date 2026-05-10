// Sync canonical Claude skill files from `.claude/skills/<name>/SKILL.md`
// into `frontend/public/skills/<name>.md` so they're downloadable from
// the agent docs page (`<a href="/skills/<name>.md" download>`).
//
// Runs on `predev` and `prebuild` so the published copies never lag the
// canonical ones — single source of truth lives in `.claude/skills/`.
//
// Add a skill to PUBLISHED_SKILLS to surface it on the docs page.

import { copyFileSync, existsSync, mkdirSync, readdirSync, unlinkSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..", "..");
const srcRoot = join(repoRoot, ".claude", "skills");
const dstRoot = join(__dirname, "..", "public", "skills");

// Skills we publish. Add new ones here as we ship them.
const PUBLISHED_SKILLS = [
  { name: "chakramcp-hermes", file: "chakramcp-hermes.md" },
];

mkdirSync(dstRoot, { recursive: true });

// Wipe any stale .md in public/skills so removed entries actually disappear.
for (const file of readdirSync(dstRoot)) {
  if (!file.endsWith(".md")) continue;
  if (PUBLISHED_SKILLS.some((s) => s.file === file)) continue;
  unlinkSync(join(dstRoot, file));
  console.log(`  rm  public/skills/${file}`);
}

let copied = 0;
for (const { name, file } of PUBLISHED_SKILLS) {
  const src = join(srcRoot, name, "SKILL.md");
  if (!existsSync(src)) {
    console.error(`  !! missing canonical skill: ${src}`);
    process.exitCode = 1;
    continue;
  }
  const dst = join(dstRoot, file);
  copyFileSync(src, dst);
  copied++;
  console.log(`  cp  ${name}/SKILL.md → public/skills/${file}`);
}

console.log(`sync-skills: ${copied}/${PUBLISHED_SKILLS.length} published`);
