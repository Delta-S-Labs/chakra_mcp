import assert from "node:assert/strict";
import { test } from "node:test";
import { openDb } from "./db.js";
import { DAY_MS } from "./limits.js";
import { Sessions } from "./sessions.js";
import { clock } from "./test-helpers.js";

const turn = [
  { role: "user" as const, content: "q" },
  { role: "assistant" as const, content: "a" },
];

test("a conversation continues until something starts it over", () => {
  const c = clock();
  const db = openDb(null);
  const s = new Sessions(db, "v1", { idleMs: 7 * DAY_MS, maxBytes: 1000 }, c.now);
  assert.deepEqual(s.load("a", false), { messages: [], turns: 0, isNew: true });
  s.save("a", turn, 1, true);
  assert.deepEqual(s.load("a", false), { messages: turn, turns: 1, isNew: false });
  // Asked for.
  assert.equal(s.load("a", true).isNew, true);
  // Another agent has its own.
  assert.equal(s.load("b", false).isNew, true);
  // A new prompt version.
  assert.equal(new Sessions(db, "v2", undefined, c.now).load("a", false).isNew, true);
  // Idle too long.
  c.advance(7 * DAY_MS + 1);
  assert.equal(s.load("a", false).isNew, true);
});

test("a conversation stored too large starts over", () => {
  const s = new Sessions(openDb(null), "v1", { idleMs: DAY_MS, maxBytes: 100 });
  s.save("a", [{ role: "user", content: "x".repeat(200) }], 1, true);
  assert.equal(s.load("a", false).isNew, true);
});

test("saving a new session replaces the old one; cleanup drops idle ones", () => {
  const c = clock();
  const db = openDb(null);
  const s = new Sessions(db, "v1", { idleMs: DAY_MS, maxBytes: 10_000 }, c.now);
  s.save("a", [...turn, ...turn], 2, true);
  s.save("a", turn, 1, true);
  assert.deepEqual(s.load("a", false), { messages: turn, turns: 1, isNew: false });
  c.advance(DAY_MS + 1);
  s.cleanup();
  assert.equal((db.prepare("SELECT COUNT(*) AS n FROM sessions").get() as { n: number }).n, 0);
});
