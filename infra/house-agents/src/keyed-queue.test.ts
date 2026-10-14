import assert from "node:assert/strict";
import { test } from "node:test";
import { KeyedLock, Semaphore } from "./keyed-queue.js";

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { promise, resolve };
}

test("one task at a time per key, in order; other keys run alongside", async () => {
  const lock = new KeyedLock();
  const order: string[] = [];
  const first = deferred();
  const a1 = lock.run("a", async () => {
    await first.promise;
    order.push("a1");
  });
  const a2 = lock.run("a", async () => void order.push("a2"));
  const b1 = lock.run("b", async () => void order.push("b1"));
  assert.equal(lock.pending("a"), 2);
  assert.equal(lock.total(), 3);
  await b1;
  assert.deepEqual(order, ["b1"]);
  first.resolve();
  await Promise.all([a1, a2]);
  assert.deepEqual(order, ["b1", "a1", "a2"]);
  await new Promise((r) => setImmediate(r));
  assert.equal(lock.total(), 0);
});

test("a failed task doesn't block the next one", async () => {
  const lock = new KeyedLock();
  await assert.rejects(lock.run("a", async () => Promise.reject(new Error("boom"))));
  assert.equal(await lock.run("a", async () => 42), 42);
});

test("the semaphore caps tasks in flight", async () => {
  const sem = new Semaphore(2);
  const r1 = await sem.acquire();
  await sem.acquire();
  let third = false;
  const p = sem.acquire().then((r) => {
    third = true;
    return r;
  });
  await new Promise((r) => setImmediate(r));
  assert.equal(third, false);
  r1();
  r1(); // releasing twice is harmless
  await p;
  assert.equal(third, true);
});
