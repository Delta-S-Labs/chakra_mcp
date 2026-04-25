/**
 * Example ChakraMCP agent — TypeScript + Mastra + NVIDIA NIM.
 *
 * Loads NVIDIA_API_KEY from .env.local at the repo root, sends a single
 * prompt to NVIDIA's OpenAI-compatible endpoint via Mastra, prints the
 * response.
 *
 * Relay registration / discovery / message-send calls live in
 * `relay-client.ts` as stubs. Once the Rust relay's Phase 1 lands (see
 * `docs/chakramcp-build-spec.md`), wire those up.
 */

import { config as dotenv } from "dotenv";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";
import { Agent } from "@mastra/core/agent";
import { createOpenAICompatible } from "@ai-sdk/openai-compatible";

import { RelayClient } from "./relay-client.js";

// Load .env.local from the repo root (three levels up from this file).
const here = dirname(fileURLToPath(import.meta.url));
dotenv({ path: resolve(here, "../../../.env.local") });
dotenv(); // also load any local .env

const SYSTEM_PROMPT =
  "You are a small, well-mannered example agent on the ChakraMCP relay " +
  "network. Answer in two short sentences.";

async function main() {
  const apiKey = process.env.NVIDIA_API_KEY;
  if (!apiKey) {
    throw new Error(
      "NVIDIA_API_KEY not set. Get a free key at https://build.nvidia.com/ and add it to .env.local",
    );
  }
  const baseURL = process.env.NVIDIA_BASE_URL ?? "https://integrate.api.nvidia.com/v1";
  const modelId = process.env.NVIDIA_MODEL ?? "meta/llama-3.1-70b-instruct";

  // OpenAI-compatible provider points at NVIDIA NIM.
  const provider = createOpenAICompatible({
    name: "nvidia-nim",
    apiKey,
    baseURL,
  });

  const agent = new Agent({
    name: "example-typescript",
    instructions: SYSTEM_PROMPT,
    model: provider(modelId),
  });

  const prompt =
    process.argv.slice(2).join(" ") ||
    "What is the relay network for AI agents in one line?";

  const result = await agent.generate([{ role: "user", content: prompt }]);
  console.log(result.text);

  // TODO: relay integration — pending Rust backend Phase 1.
  // const relay = new RelayClient(process.env.RELAY_URL ?? "http://localhost:8080");
  // await relay.registerAgent({ name: "example-ts", capabilities: ["echo"] });
  // for (const event of await relay.pollEvents()) { ... }
  void RelayClient; // suppress unused-import warning until Phase 1 lands
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
