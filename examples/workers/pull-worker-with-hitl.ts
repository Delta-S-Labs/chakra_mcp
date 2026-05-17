#!/usr/bin/env node
/**
 * Reference pull-mode worker — autonomous + human-in-the-loop (TypeScript).
 *
 * Twin of `pull_worker_with_hitl.py`. One `chakra.inbox.serve()` loop
 * handles two capabilities:
 *
 *   * `propose_slots`   — autonomous. `handler()` returns a result
 *                         envelope; the SDK posts it back.
 *   * `message_owner`   — human-in-the-loop. `humanHandler()` runs for
 *                         side-effects only — the SDK does NOT post a
 *                         result. The row stays `in_progress` until a
 *                         human resolves it out-of-band via:
 *                            chakramcp message reply <id> "<reply_text>"
 *                         which posts the wire result with
 *                         `confirmed_by_human: true` and satisfies the
 *                         relay's HITL gate (issue #69 PR 2).
 *
 * `humanHandler()` writes each pending invocation to
 * `./pending/<invocationId>.json` and prints a one-line summary to
 * stderr. A human operator (or a higher-level UI watching the
 * directory) reads the files and replies via the CLI.
 *
 * Bonus: at startup the worker can fire one outbound `message_owner`
 * via cli-v0.1.2's `chakramcp invoke ensure` (issue #68) — the
 * autonomous-orchestration sugar that does discover + ensure-friendship
 * + ensure-grant + invoke in one shot. We parse the `--json` output
 * with `node:child_process` so the worker can react programmatically.
 * Enable with `CHAKRAMCP_PING_PEER=<peer-slug>`.
 *
 * Configuration (env vars):
 *   CHAKRAMCP_API_KEY      required
 *   CHAKRAMCP_AGENT_ID     required
 *   CHAKRAMCP_AGENT_SLUG   required when CHAKRAMCP_PING_PEER is set
 *   CHAKRAMCP_APP_URL      optional
 *   CHAKRAMCP_RELAY_URL    optional
 *   CHAKRAMCP_PUBLISH      optional — "1" to publish both capabilities
 *   CHAKRAMCP_PENDING_DIR  optional — defaults to "./pending"
 *   CHAKRAMCP_PING_PEER    optional — `<account>/<slug>` to ping
 *   CHAKRAMCP_PING_TEXT    optional — defaults to "hello from the reference worker"
 *
 * Run:
 *   npm install
 *   node --loader ts-node/esm pull-worker-with-hitl.ts
 *
 * Stop with ctrl-c.
 */

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

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

function pendingDir(): string {
  const dir = resolve(process.env.CHAKRAMCP_PENDING_DIR ?? "./pending");
  mkdirSync(dir, { recursive: true });
  return dir;
}

function fakeProposeSlots(_durationMin: number, withinDays: number): string[] {
  const now = new Date();
  now.setUTCMilliseconds(0);
  now.setUTCSeconds(0);
  const slots: string[] = [];
  for (let i = 0; i < 4; i++) {
    const daysOut = Math.max(1, Math.floor(Math.random() * Math.max(1, withinDays)) + 1);
    const hour = 9 + Math.floor(Math.random() * 8);
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

  if (capability === "propose_slots") {
    const slots = fakeProposeSlots(
      Number(inputs.duration_min ?? 30),
      Number(inputs.within_days ?? 7),
    );
    console.log(`  → returning ${slots.length} slots`);
    return { status: "succeeded", output: { slots } };
  }

  return { status: "failed", error: `unsupported capability: ${capability}` };
}

async function humanHandler(invocation: Invocation): Promise<void> {
  // HITL path. Side-effects only — MUST NOT post a result. The row
  // stays `in_progress` until a human runs `chakramcp message reply`.
  const invId = invocation.id;
  const payload = (invocation.input_preview ?? {}) as Record<string, unknown>;
  const message = String(payload.message ?? "");
  const urgency = String(payload.urgency ?? "normal");
  const fromName =
    (payload.from_display_name as string | undefined) ??
    invocation.grantee_display_name ??
    "(unknown)";

  const outPath = resolve(pendingDir(), `${invId}.json`);
  writeFileSync(outPath, JSON.stringify(invocation, null, 2));

  // One-line summary so `tail -F` is enough to monitor traffic.
  process.stderr.write(
    `[HITL] ${invId}  from=${JSON.stringify(fromName)}  urgency=${urgency}  ` +
      `msg=${JSON.stringify(message.slice(0, 80))}  → ${outPath}\n`,
  );
}

async function ensureCapabilities(chakra: ChakraMCP, agentId: string): Promise<void> {
  if (process.env.CHAKRAMCP_PUBLISH !== "1") return;
  try {
    await chakra.agents.capabilities.create(agentId, PROPOSE_SLOTS_CAPABILITY);
    console.log(`  published: ${PROPOSE_SLOTS_CAPABILITY.name} (autonomous)`);
  } catch (err) {
    if (err instanceof ChakraMCPError) {
      console.error(`  publish skipped (propose_slots): ${err.message}`);
    } else {
      throw err;
    }
  }
  try {
    await chakra.agents.capabilities.addTemplate(agentId, "message_owner");
    console.log("  published: message_owner (human_in_loop)");
  } catch (err) {
    if (err instanceof ChakraMCPError) {
      console.error(`  publish skipped (message_owner): ${err.message}`);
    } else {
      throw err;
    }
  }
}

function pingPeerViaCli(peer: string, text: string, fromSlug: string): void {
  // Outbound bonus: cli-v0.1.2's `invoke ensure` is the autonomous
  // orchestration primitive — one call discovers the peer, ensures
  // friendship + grant, invokes, optionally waits. We pass `--json`
  // so we can parse the structured response.
  //
  // `message_owner` is HITL on the peer's side, so we explicitly do
  // NOT pass `--wait` — the row will stay `in_progress` until their
  // human replies. A real worker picks the reply up later via its
  // own inbox.
  const args = [
    "invoke",
    "ensure",
    peer,
    "message_owner",
    JSON.stringify({ message: text, urgency: "normal" }),
    "--from",
    fromSlug,
    "--wait-for-friendship",
    "--wait-for-grant",
    "--json",
  ];
  console.log(`  ⇢ outbound: chakramcp ${args.join(" ")}`);

  const proc = spawnSync("chakramcp", args, { encoding: "utf8", timeout: 120_000 });

  if (proc.error) {
    const code = (proc.error as NodeJS.ErrnoException).code;
    if (code === "ENOENT") {
      process.stderr.write(
        "  outbound skipped: `chakramcp` CLI not on PATH " +
          "(install with `brew install chakramcp` or `cargo install chakramcp-cli`).\n",
      );
      return;
    }
    process.stderr.write(`  outbound spawn failed: ${proc.error.message}\n`);
    return;
  }

  if (proc.status !== 0) {
    // `invoke ensure` exits non-zero on waiting_for_* states too — the
    // JSON body still tells us what happened.
    process.stderr.write(
      `  outbound exit=${proc.status} stderr=${JSON.stringify(
        (proc.stderr ?? "").trim().slice(0, 200),
      )}\n`,
    );
  }

  try {
    const body = JSON.parse(proc.stdout) as {
      ok?: boolean;
      invocation?: { id?: string; status?: string };
    };
    const inv = body.invocation ?? {};
    console.log(
      `  outbound: ok=${body.ok ?? false} invocation_id=${inv.id ?? "n/a"} status=${inv.status ?? "n/a"}`,
    );
  } catch {
    process.stderr.write(
      `  outbound: non-JSON stdout: ${JSON.stringify((proc.stdout ?? "").slice(0, 200))}\n`,
    );
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
  console.log(`pending: ${pendingDir()}  (HITL drop directory)`);
  await ensureCapabilities(chakra, agentId);

  const peer = process.env.CHAKRAMCP_PING_PEER;
  const fromSlug = process.env.CHAKRAMCP_AGENT_SLUG;
  if (peer && fromSlug) {
    pingPeerViaCli(
      peer,
      process.env.CHAKRAMCP_PING_TEXT ?? "hello from the reference worker",
      fromSlug,
    );
  } else if (peer && !fromSlug) {
    process.stderr.write(
      "  outbound skipped: CHAKRAMCP_PING_PEER set but " +
        "CHAKRAMCP_AGENT_SLUG is missing (needed for --from).\n",
    );
  }

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
      humanHandler,
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
