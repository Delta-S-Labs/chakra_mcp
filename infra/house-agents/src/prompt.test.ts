import assert from "node:assert/strict";
import { test } from "node:test";
import { costUsd, worstCaseUsd } from "./pricing.js";
import { buildSystemPrompt, promptVersion } from "./system-prompt.js";

test("the system prompt carries every doc; its version tracks prompt and model", () => {
  const system = buildSystemPrompt([
    { path: "README.md", text: "Read me." },
    { path: "docs/INSTALL.md", text: "Install." },
  ]);
  assert.match(system, /<document path="README.md">\nRead me.\n<\/document>/);
  assert.match(system, /<document path="docs\/INSTALL.md">/);
  const v = promptVersion(system, "claude-sonnet-5-5");
  assert.equal(v, promptVersion(system, "claude-sonnet-5-5"));
  assert.notEqual(v, promptVersion(system + " ", "claude-sonnet-5-5"));
  assert.notEqual(v, promptVersion(system, "claude-opus-5-5"));
});

test("cost counts cache writes and reads, and compaction iterations", () => {
  assert.equal(
    costUsd({ input_tokens: 1_000_000, output_tokens: 0, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 }),
    2,
  );
  assert.equal(costUsd({ cache_creation_input_tokens: 1_000_000, cache_read_input_tokens: 1_000_000 }), 2.7);
  assert.equal(
    costUsd({
      input_tokens: 10,
      iterations: [
        { type: "compaction", input_tokens: 1_000_000, output_tokens: 0 },
        { type: "message", input_tokens: 0, output_tokens: 1_000_000 },
      ],
    }),
    12,
  );
  assert.ok(worstCaseUsd(300_000, 3000) > costUsd({ cache_creation_input_tokens: 100_000, output_tokens: 3000 }));
});
