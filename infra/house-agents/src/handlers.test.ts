import assert from "node:assert/strict";
import { test } from "node:test";
import type { HouseAgent } from "./agents.js";
import { openDb } from "./db.js";
import {
  guideOutput,
  handleGuide,
  handlePing,
  OUTPUT_MAX_BYTES,
  parseGuideInput,
  pingOutput,
  type GuideDeps,
} from "./handlers.js";
import { KeyedLock, Semaphore } from "./keyed-queue.js";
import { guideLimits, Limiter } from "./limits.js";
import { Sessions } from "./sessions.js";
import { fakeChakra, fakeClaude, invocation, reply } from "./test-helpers.js";

const GUIDE: HouseAgent = { id: "guide-id", slug: "guide", capabilityId: "cap-guide", capabilityName: "guide.ask" };
const PING: HouseAgent = { id: "ping-id", slug: "ping", capabilityId: "cap-ping", capabilityName: "ping.echo" };

test("Guide input: a question, optionally new_session", () => {
  assert.deepEqual(parseGuideInput({ question: "  hi  " }), { question: "hi", newSession: false });
  assert.deepEqual(parseGuideInput({ question: "hi", new_session: true }), { question: "hi", newSession: true });
  assert.deepEqual(parseGuideInput("plain text"), { question: "plain text", newSession: false });
  assert.equal(typeof parseGuideInput(null), "string");
  assert.equal(typeof parseGuideInput({ question: "" }), "string");
  assert.equal(typeof parseGuideInput({ question: "x".repeat(4001) }), "string");
  assert.equal(typeof parseGuideInput({ question: "hi", new_session: "yes" }), "string");
  assert.match(parseGuideInput({ __chakramcp_truncated__: true }) as string, /too large/);
  assert.match(parseGuideInput({ question: "hi", padding: "x".repeat(9000) }) as string, /too large/);
});

test("Guide input from an A2A message: text parts, or a data part", () => {
  const msg = (parts: unknown[]) => ({ message: { role: "user", messageId: "m1", parts } });
  assert.deepEqual(parseGuideInput(msg([{ kind: "text", text: "How do grants" }, { kind: "text", text: "work?" }])), {
    question: "How do grants\nwork?",
    newSession: false,
  });
  assert.deepEqual(parseGuideInput(msg([{ kind: "data", data: { question: "hi", new_session: true } }])), {
    question: "hi",
    newSession: true,
  });
  assert.deepEqual(parseGuideInput({ message: "hi" }), { question: "hi", newSession: false });
  assert.equal(typeof parseGuideInput(msg([{ kind: "file", file: {} }])), "string");
  assert.deepEqual(pingOutput(msg([{ type: "text", text: "yo" }]), new Date(0)).message, "yo");
});

test("Guide output fits in 12 KB", () => {
  const out = guideOutput("é".repeat(20_000), false, 3, false);
  assert.ok(Buffer.byteLength(JSON.stringify(out)) <= OUTPUT_MAX_BYTES);
  assert.equal(out.cut_short, true);
  assert.match(out.answer as string, /Cut to fit/);
  assert.deepEqual(guideOutput("short", false, 1, true), {
    answer: "short",
    cut_short: false,
    conversation: { turn: 1, new_session: true },
  });
});

test("Ping echoes up to 500 characters", () => {
  const out = pingOutput({ message: "x".repeat(600) }, new Date(0));
  assert.equal((out.message as string).length, 500);
  assert.deepEqual(pingOutput(null, new Date(0)), { pong: true, answered_at: new Date(0).toISOString() });
});

test("Ping answers, then refuses blocked agents and floods", async () => {
  const { chakra, results } = fakeChakra();
  const limiter = new Limiter(openDb(null));
  const deps = { chakra, house: PING, limiter, blocked: { agents: new Set(["bad-agent"]), accounts: new Set<string>() } };
  const ok = invocation({ capability_name: "ping.echo", input_preview: { message: "hi" } });
  await handlePing(deps, ok);
  assert.equal(results.get(ok.id)?.status, "succeeded");

  const blocked = invocation({ capability_name: "ping.echo", grantee_agent_id: "BAD-AGENT" });
  await handlePing(deps, blocked);
  assert.equal(results.get(blocked.id)?.status, "failed");

  for (let i = 0; i < 99; i++) await handlePing(deps, invocation({ capability_name: "ping.echo" }));
  const over = invocation({ capability_name: "ping.echo" });
  await handlePing(deps, over);
  const res = results.get(over.id);
  assert.ok(res?.status === "failed" && /Limit reached/.test(res.error));
});

function guideDeps(script: Parameters<typeof fakeClaude>[0], over: Partial<GuideDeps> = {}) {
  const { chakra, results } = fakeChakra();
  const db = openDb(null);
  const claude = fakeClaude(script);
  const deps: GuideDeps = {
    chakra,
    house: GUIDE,
    limiter: new Limiter(db),
    limits: guideLimits(10),
    sessions: new Sessions(db, "v1"),
    claude,
    blocked: { agents: new Set(), accounts: new Set() },
    lock: new KeyedLock(),
    slots: new Semaphore(2),
    stopping: () => false,
    ...over,
  };
  return { deps, results, claude };
}

test("the Guide answers and records the cost against the caller", async () => {
  const { deps, results } = guideDeps([reply("Use chakramcp agents create.")]);
  const inv = invocation();
  await handleGuide(deps, inv);
  const res = results.get(inv.id);
  assert.ok(res?.status === "succeeded");
  assert.equal(res.output.answer, "Use chakramcp agents create.");
  assert.ok(deps.limiter.spentToday("guide.ask") > 0);
});

test("the Guide refuses bad input, the wrong capability and a third pending question", async () => {
  const { deps, results, claude } = guideDeps([reply("a"), reply("b")]);
  const bad = invocation({ input_preview: { nope: 1 } });
  const wrong = invocation({ capability_name: "guide.other" });
  await handleGuide(deps, bad);
  await handleGuide(deps, wrong);
  assert.equal(results.get(bad.id)?.status, "failed");
  assert.equal(results.get(wrong.id)?.status, "failed");

  const first = handleGuide(deps, invocation());
  const second = handleGuide(deps, invocation());
  const third = invocation();
  await handleGuide(deps, third);
  const res = results.get(third.id);
  assert.ok(res?.status === "failed" && /One question at a time/.test(res.error));
  await Promise.all([first, second]);
  assert.equal(claude.calls.length, 2);
});

test("the 11th question in an hour is refused without calling Claude", async () => {
  const { deps, results, claude } = guideDeps(Array.from({ length: 10 }, () => reply("ok")));
  for (let i = 0; i < 10; i++) await handleGuide(deps, invocation());
  const eleventh = invocation();
  await handleGuide(deps, eleventh);
  const res = results.get(eleventh.id);
  assert.ok(res?.status === "failed" && /10 calls an hour/.test(res.error));
  assert.equal(claude.calls.length, 10);
});

test("questions still queued at shutdown are turned away", async () => {
  let stopping = false;
  const { deps, results, claude } = guideDeps([], { stopping: () => stopping });
  stopping = true;
  const inv = invocation();
  await handleGuide(deps, inv);
  const res = results.get(inv.id);
  assert.ok(res?.status === "failed" && /restarting/.test(res.error));
  assert.equal(claude.calls.length, 0);
});

test("a Claude failure is posted as a failed result", async () => {
  const { deps, results } = guideDeps([new Error("socket hang up")]);
  const inv = invocation();
  await handleGuide(deps, inv);
  assert.equal(results.get(inv.id)?.status, "failed");
});
