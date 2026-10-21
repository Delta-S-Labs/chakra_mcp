import assert from "node:assert/strict";
import { test } from "node:test";
import Anthropic from "@anthropic-ai/sdk";
import { openDb } from "./db.js";
import { ask, GuideError } from "./guide.js";
import { Sessions } from "./sessions.js";
import { fakeClaude, reply } from "./test-helpers.js";

const badRequest = () =>
  new Anthropic.BadRequestError(400, { type: "error", error: { type: "invalid_request_error" } }, "bad", new Headers());

test("each agent's conversation is replayed whole and appended to", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const claude = fakeClaude([reply("first answer"), reply("second answer"), reply("other agent")]);
  const costs: number[] = [];
  const a1 = await ask(claude, sessions, "a", "q1", false, (usd) => costs.push(usd));
  assert.deepEqual(a1, { text: "first answer", cutShort: false, turn: 1, newSession: true });
  const a2 = await ask(claude, sessions, "a", "q2", false, () => {});
  assert.equal(a2.turn, 2);
  assert.equal(a2.newSession, false);
  // The second request starts with the first turn, unchanged.
  assert.equal(claude.calls[1]!.length, 3);
  assert.deepEqual(claude.calls[1]!.slice(0, 1), claude.calls[0]);
  assert.equal(claude.calls[1]![1]!.role, "assistant");
  // Another agent starts fresh.
  await ask(claude, sessions, "b", "q", false, () => {});
  assert.equal(claude.calls[2]!.length, 1);
  // 1000 input tokens at $2/M + 100 output at $10/M.
  assert.equal(costs[0]!.toFixed(4), "0.0030");
});

test("compaction blocks are kept in the stored history", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const compacted = reply("answer");
  compacted.content.unshift({ type: "compaction", content: "summary", encrypted_content: null } as never);
  const claude = fakeClaude([compacted, reply("next")]);
  await ask(claude, sessions, "a", "q1", false, () => {});
  await ask(claude, sessions, "a", "q2", false, () => {});
  const assistant = claude.calls[1]![1]!;
  assert.ok(Array.isArray(assistant.content) && assistant.content[0]!.type === "compaction");
});

test("new_session starts over", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const claude = fakeClaude([reply("1"), reply("2")]);
  await ask(claude, sessions, "a", "q1", false, () => {});
  const a = await ask(claude, sessions, "a", "q2", true, () => {});
  assert.equal(claude.calls[1]!.length, 1);
  assert.deepEqual([a.turn, a.newSession], [1, true]);
});

test("a history that no longer replays starts over once", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const claude = fakeClaude([reply("1"), badRequest(), reply("fresh")]);
  await ask(claude, sessions, "a", "q1", false, () => {});
  const a = await ask(claude, sessions, "a", "q2", false, () => {});
  assert.equal(a.text, "fresh");
  assert.equal(a.newSession, true);
  assert.equal(claude.calls[2]!.length, 1);
});

test("a bad request on a fresh conversation isn't retried", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const claude = fakeClaude([badRequest()]);
  await assert.rejects(ask(claude, sessions, "a", "q", false, () => {}), GuideError);
  assert.equal(claude.calls.length, 1);
});

test("a refusal isn't saved", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const claude = fakeClaude([reply("1"), reply("", { stop_reason: "refusal" }), reply("2")]);
  await ask(claude, sessions, "a", "q1", false, () => {});
  await assert.rejects(ask(claude, sessions, "a", "bad", false, () => {}), /can't help/);
  await ask(claude, sessions, "a", "q3", false, () => {});
  // q1, its answer, q3: the refused question is gone.
  assert.equal(claude.calls[2]!.length, 3);
});

test("failures cost nothing when rejected, a worst case otherwise", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const costs: number[] = [];
  const rate = new Anthropic.RateLimitError(429, undefined, "slow down", new Headers());
  await assert.rejects(
    ask(fakeClaude([rate]), sessions, "a", "q", false, (usd) => costs.push(usd)),
    (err: GuideError) => err.retryAfterSecs === 60,
  );
  await assert.rejects(
    ask(fakeClaude([new Anthropic.APIConnectionTimeoutError()]), sessions, "a", "q", false, (usd) => costs.push(usd)),
    /too long/,
  );
  assert.equal(costs[0], 0);
  assert.ok(costs[1]! > 0.05);
});

test("an answer cut off by the output limit says so", async () => {
  const sessions = new Sessions(openDb(null), "v1");
  const a = await ask(fakeClaude([reply("partial", { stop_reason: "max_tokens" })]), sessions, "a", "q", false, () => {});
  assert.equal(a.cutShort, true);
});
