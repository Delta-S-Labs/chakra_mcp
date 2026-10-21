// One real question to Claude, opt-in: it spends a few cents. Run with
//   npm run build && HOUSE_LIVE_TEST=1 HOUSE_ANTHROPIC_API_KEY=... npm test
// Checks that the request shape is accepted and that the second turn of a
// conversation reads the prompt cache.
import assert from "node:assert/strict";
import { test } from "node:test";
import Anthropic from "@anthropic-ai/sdk";
import type { BetaMessageParam } from "@anthropic-ai/sdk/resources/beta/messages/messages";
import { makeClaude } from "./guide.js";
import { costUsd } from "./pricing.js";
import { buildSystemPrompt, loadDocs } from "./system-prompt.js";

const live = process.env.HOUSE_LIVE_TEST === "1";

test("a real conversation is accepted and cached", { skip: !live && "set HOUSE_LIVE_TEST=1" }, async () => {
  const claude = makeClaude(
    new Anthropic({ apiKey: process.env.HOUSE_ANTHROPIC_API_KEY, maxRetries: 1 }),
    process.env.HOUSE_MODEL ?? "claude-sonnet-5-5",
    buildSystemPrompt(loadDocs()),
  );
  assert.ok((await claude.check()) > 10_000, "the docs should be in the system prompt");

  const messages: BetaMessageParam[] = [{ role: "user", content: [{ type: "text", text: "In one sentence: what is a grant?" }] }];
  const first = await claude.call(messages);
  assert.equal(first.stop_reason, "end_turn");
  messages.push({ role: "assistant", content: first.content });
  messages.push({ role: "user", content: [{ type: "text", text: "And a friendship, in one sentence?" }] });
  const second = await claude.call(messages);
  assert.ok((second.usage.cache_read_input_tokens ?? 0) > 0, "the second turn should read the cache");
  console.log(`cost: first $${costUsd(first.usage).toFixed(4)}, second $${costUsd(second.usage).toFixed(4)}`);
});
