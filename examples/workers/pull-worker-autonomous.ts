#!/usr/bin/env node
/**
 * Reference pull-mode worker — autonomous capability only (TypeScript).
 *
 * TypeScript twin of `pull_worker_autonomous.py`. Registers (or reuses)
 * an agent, optionally publishes a `propose_slots` capability with
 * `semantics: "autonomous"` set explicitly, and runs `chakra.inbox.serve()`
 * forever. Every invocation is handled by the SDK's autonomous path —
 * the handler returns a `HandlerResult` envelope and the SDK posts it
 * back to the relay.
 *
 * Configuration (env vars):
 *   CHAKRAMCP_API_KEY      required — `ck_…` from `chakramcp keys create`
 *   CHAKRAMCP_AGENT_ID     required — the agent that will serve the inbox
 *   CHAKRAMCP_APP_URL      optional — defaults to https://chakramcp.com
 *   CHAKRAMCP_RELAY_URL    optional — defaults to https://relay.chakramcp.com
 *   CHAKRAMCP_PUBLISH      optional — set to "1" to (re-)publish the
 *                          `propose_slots` capability on startup.
 *
 * Run:
 *   export CHAKRAMCP_API_KEY=ck_…
 *   export CHAKRAMCP_AGENT_ID=01HXXXXXXXXXXXXXXXXXXXXXXX
 *   npm install
 *   node --loader ts-node/esm pull-worker-autonomous.ts
 *
 * Stop with ctrl-c.
 */

import {
  ChakraMCP,
  ChakraMCPError,
  type CreateCapabilityRequest,
  type HandlerResult,
  type Invocation,
} from "@chakramcp/sdk";

const PROPOSE_SLOTS_CAPABILITY: CreateCapabilityRequest = {
  name: "propose_slots",
  description: "Suggest up to four meeting slots in the next N days.",
  // Explicit even though "autonomous" is the relay-side default — the
  // whole point of the reference is to make the HITL decision visible.
  semantics: "autonomous",
  input_schema: {
    type: "object",
    properties: {
      duration_min: { type: "integer", minimum: 5, maximum: 480, default: 30 },
      within_days: { type: "integer", minimum: 1, maximum: 60, default: 7 },
    },
  },
  output_schema: {
    type: "object",
    required: ["slots"],
    properties: {
      slots: { type: "array", items: { type: "string", format: "date-time" } },
    },
  },
  visibility: "network",
};

function fakeProposeSlots(_durationMin: number, withinDays: number): string[] {
  const now = new Date();
  now.setUTCMilliseconds(0);
  now.setUTCSeconds(0);
  const slots: string[] = [];
  for (let i = 0; i < 4; i++) {
    const daysOut = Math.max(1, Math.floor(Math.random() * Math.max(1, withinDays)) + 1);
    const hour = 9 + Math.floor(Math.random() * 8); // 9..16
    const ts = new Date(now);
    ts.setUTCDate(ts.getUTCDate() + daysOut);
    ts.setUTCHours(hour, 0, 0, 0);
    slots.push(ts.toISOString());
  }
  slots.sort();
  return slots;
}

async function handle(invocation: Invocation): Promise<HandlerResult> {
  const capability = invocation.capability_name;
  const inputs = (invocation.input_preview ?? {}) as Record<string, unknown>;
  console.log(`  ← ${capability}(${JSON.stringify(inputs)})`);

  if (capability !== "propose_slots") {
    return { status: "failed", error: `unsupported capability: ${capability}` };
  }

  const slots = fakeProposeSlots(
    Number(inputs.duration_min ?? 30),
    Number(inputs.within_days ?? 7),
  );
  console.log(`  → returning ${slots.length} slots`);
  return { status: "succeeded", output: { slots } };
}

async function ensureCapability(chakra: ChakraMCP, agentId: string): Promise<void> {
  if (process.env.CHAKRAMCP_PUBLISH !== "1") return;
  try {
    await chakra.agents.capabilities.create(agentId, PROPOSE_SLOTS_CAPABILITY);
    console.log(`  published capability: ${PROPOSE_SLOTS_CAPABILITY.name}`);
  } catch (err) {
    if (err instanceof ChakraMCPError) {
      console.error(`  capability publish skipped: ${err.message}`);
    } else {
      throw err;
    }
  }
}

async function main(): Promise<number> {
  const apiKey = process.env.CHAKRAMCP_API_KEY;
  const agentId = process.env.CHAKRAMCP_AGENT_ID;
  if (!apiKey || !agentId) {
    console.error("error: CHAKRAMCP_API_KEY and CHAKRAMCP_AGENT_ID must be set.");
    return 2;
  }

  const chakra = new ChakraMCP({
    apiKey,
    appUrl: process.env.CHAKRAMCP_APP_URL,
    relayUrl: process.env.CHAKRAMCP_RELAY_URL,
  });

  const me = await chakra.me();
  console.log(`signed in as ${me.user.email}`);
  console.log(`agent  : ${agentId}`);
  await ensureCapability(chakra, agentId);
  console.log();
  console.log("Listening for invocations… (ctrl-c to stop)");
  console.log();

  const ac = new AbortController();
  const onSignal = () => ac.abort();
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  try {
    await chakra.inbox.serve(agentId, {
      handler: handle,
      pollIntervalMs: 2000,
      signal: ac.signal,
      onError: (err, inv) =>
        console.error(`  ! error: ${String(err)} (inv=${inv?.id ?? "n/a"})`),
    });
  } finally {
    process.off("SIGINT", onSignal);
    process.off("SIGTERM", onSignal);
  }

  console.log();
  console.log("stopped.");
  return 0;
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(err);
    process.exit(1);
  });
