import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";
import { test } from "node:test";
import type { ChakraMCP, PullOptions } from "@chakramcp/sdk";
import { PULL_WAIT_SECS, pullLoop, type HouseInvocation } from "./relay.js";
import { invocation } from "./test-helpers.js";

type Pulled = { items: HouseInvocation[]; waited: number | null };

/** A relay whose pulls answer from a script, then hang until aborted. */
function scriptedRelay(script: Pulled[]): { chakra: ChakraMCP; pulls: PullOptions[] } {
  const pulls: PullOptions[] = [];
  const chakra = {
    inbox: {
      async pullWithMeta(_agentId: string, opts: PullOptions): Promise<Pulled> {
        pulls.push({ limit: opts.limit, wait: opts.wait });
        const next = script.shift();
        if (next) return next;
        return new Promise((_, reject) =>
          opts.signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true }),
        );
      },
    },
  } as unknown as ChakraMCP;
  return { chakra, pulls };
}

test("pulls wait on the relay, and pull again straight after a wait", async () => {
  const inv = invocation();
  const { chakra, pulls } = scriptedRelay([
    { items: [], waited: PULL_WAIT_SECS },
    { items: [inv], waited: 3 },
  ]);
  const got: HouseInvocation[] = [];
  const stop = new AbortController();
  const loop = pullLoop({ chakra, agentId: "a", room: () => 4, onInvocation: (i) => got.push(i), signal: stop.signal });
  await sleep(50);
  // Both scripted pulls ran without a pause, and the third is waiting.
  assert.deepEqual(pulls, [
    { limit: 4, wait: PULL_WAIT_SECS },
    { limit: 4, wait: PULL_WAIT_SECS },
    { limit: 4, wait: PULL_WAIT_SECS },
  ]);
  assert.deepEqual(got, [inv]);
  stop.abort();
  await loop;
});

test("a relay that didn't wait is polled, not hammered", async () => {
  const { chakra, pulls } = scriptedRelay([{ items: [], waited: null }]);
  const stop = new AbortController();
  const loop = pullLoop({ chakra, agentId: "a", room: () => 1, onInvocation: () => {}, signal: stop.signal });
  await sleep(50);
  assert.equal(pulls.length, 1);
  stop.abort();
  await loop;
});
