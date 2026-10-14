// Talking to the relay: pulling an agent's inbox and posting results, with
// retries, so one blip doesn't leave an invocation hanging.

import { setTimeout as sleep } from "node:timers/promises";
import { ChakraMCPError, type ChakraMCP, type Invocation } from "@chakramcp/sdk";
import { errorName, log } from "./log.js";

/** An inbox row; the relay adds the caller's account on pulls. */
export type HouseInvocation = Invocation & { grantee_account_id?: string | null };

export type Outcome =
  | { status: "succeeded"; output: Record<string, unknown> }
  | { status: "failed"; error: string };

export function failed(error: string): Outcome {
  return { status: "failed", error };
}

/** Waits, unless `signal` aborts first. Never throws. */
export async function pause(ms: number, signal?: AbortSignal): Promise<void> {
  try {
    await sleep(ms, undefined, { signal });
  } catch {
    // aborted
  }
}

function retryable(err: unknown): boolean {
  if (err instanceof ChakraMCPError) {
    return err.status >= 500 || err.status === 408 || err.status === 429;
  }
  return true; // network errors
}

/** Posts a result, retrying transient failures until `deadlineMs` passes. */
export async function respond(
  chakra: ChakraMCP,
  inv: HouseInvocation,
  outcome: Outcome,
  deadlineMs: number,
): Promise<boolean> {
  const end = Date.now() + deadlineMs;
  let delay = 500;
  for (;;) {
    try {
      await chakra.inbox.respond(inv.id, outcome);
      return true;
    } catch (err) {
      if (!retryable(err)) {
        log.warn("respond.rejected", {
          invocation_id: inv.id,
          status: err instanceof ChakraMCPError ? err.status : null,
        });
        return false;
      }
      if (Date.now() + delay > end) {
        log.error("respond.gave_up", { invocation_id: inv.id, error: errorName(err) });
        return false;
      }
      await pause(delay);
      delay = Math.min(delay * 2, 10_000);
    }
  }
}

export interface PullLoop {
  chakra: ChakraMCP;
  agentId: string;
  /** How many more invocations this agent can take right now. */
  room: () => number;
  onInvocation: (inv: HouseInvocation) => void;
  signal: AbortSignal;
}

const IDLE_POLL_MS = 5000;
const BUSY_POLL_MS = 250;

/**
 * Claims an agent's invocations until `signal` aborts. Polls quickly while
 * work arrives and every few seconds when idle; each poll also keeps the
 * agent "online" in the directory.
 */
export async function pullLoop(loop: PullLoop): Promise<void> {
  let backoff = 1000;
  while (!loop.signal.aborted) {
    const room = Math.min(loop.room(), 10);
    if (room <= 0) {
      await pause(1000, loop.signal);
      continue;
    }
    try {
      const invs = (await loop.chakra.inbox.pull(loop.agentId, { limit: room })) as HouseInvocation[];
      backoff = 1000;
      for (const inv of invs) loop.onInvocation(inv);
      await pause(invs.length > 0 ? BUSY_POLL_MS : IDLE_POLL_MS, loop.signal);
    } catch (err) {
      log.warn("inbox.pull_failed", {
        agent_id: loop.agentId,
        error: errorName(err),
        status: err instanceof ChakraMCPError ? err.status : null,
      });
      await pause(backoff, loop.signal);
      backoff = Math.min(backoff * 2, 30_000);
    }
  }
}
