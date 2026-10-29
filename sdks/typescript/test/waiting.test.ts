/**
 * Waiting pulls (relay 0.7+): `wait` asks the relay to hold a pull open
 * for work, `ChakraMCP-Wait` says how long it did, and the serve loops
 * pull again at once after a wait but sleep between empty pulls from
 * servers that don't wait.
 */
import { describe, expect, it } from "vitest";
import { ChakraMCP } from "../src/index.js";

interface Pull {
  path: string;
  query: Record<string, string>;
}

/** A relay whose feeds always answer `[]`, with `header` when given. */
function emptyRelay(header?: string): { sdk: ChakraMCP; pulls: Pull[] } {
  const pulls: Pull[] = [];
  const sdk = new ChakraMCP({
    apiKey: "ck_test",
    relayUrl: "http://relay",
    fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(new Request(input, init).url);
      pulls.push({ path: url.pathname, query: Object.fromEntries(url.searchParams) });
      const headers: Record<string, string> = { "content-type": "application/json" };
      if (header !== undefined) headers["ChakraMCP-Wait"] = header;
      // A real pull is I/O: let timers run, as the network would.
      await new Promise((r) => setTimeout(r, 2));
      return new Response("[]", { status: 200, headers });
    }) as typeof fetch,
  });
  return { sdk, pulls };
}

/** Run `loop` for `ms`, then abort it; resolves when the loop has returned. */
async function runFor(ms: number, loop: (signal: AbortSignal) => Promise<void>): Promise<number> {
  const ac = new AbortController();
  const started = Date.now();
  const done = loop(ac.signal);
  await new Promise((r) => setTimeout(r, ms));
  ac.abort();
  await done;
  return Date.now() - started;
}

const handler = async () => ({ status: "succeeded" as const, output: {} });

describe("waiting pulls", () => {
  it("pull sends wait and lease_secs, and pullWithMeta reads the header", async () => {
    const r = emptyRelay("25");
    const got = await r.sdk.inbox.pullWithMeta("agent-1", { limit: 5, wait: 25, leaseSecs: 600 });
    expect(got).toEqual({ items: [], waited: 25 });
    expect(r.pulls[0]).toEqual({
      path: "/v1/inbox",
      query: { agent_id: "agent-1", limit: "5", wait: "25", lease_secs: "600" },
    });
    expect(await r.sdk.inbox.pull("agent-1")).toEqual([]);
    expect(r.pulls[1]!.query).toEqual({ agent_id: "agent-1" });
  });

  it("waited is null from a server that sends no header", async () => {
    const r = emptyRelay();
    expect(await r.sdk.inbox.pullWithMeta("agent-1", { wait: 25 })).toEqual({ items: [], waited: null });
    expect(await r.sdk.calls.pullWithMeta("agent-1", { wait: 25 })).toEqual({ items: [], waited: null });
    expect(r.pulls[1]).toEqual({ path: "/v1/inbox/calls", query: { agent_id: "agent-1", wait: "25" } });
  });

  it("notices take wait", async () => {
    const r = emptyRelay("10");
    await r.sdk.inbox.notices("agent-1", { wait: 10 });
    expect(r.pulls[0]).toEqual({ path: "/v1/inbox/notices", query: { agent_id: "agent-1", wait: "10" } });
  });

  it("inbox.serve pulls again at once while the relay waits", async () => {
    const r = emptyRelay("25");
    const took = await runFor(60, (signal) =>
      r.sdk.inbox.serve("agent-1", {
        handler,
        noticeIntervalMs: 0,
        leaseSecs: 900,
        pollIntervalMs: 30_000,
        signal,
      }),
    );
    expect(took).toBeLessThan(1000);
    expect(r.pulls.length).toBeGreaterThan(3);
    expect(r.pulls[0]!.query).toEqual({ agent_id: "agent-1", limit: "25", wait: "25", lease_secs: "900" });
  });

  it("inbox.serve waits no longer than the next notices check", async () => {
    const r = emptyRelay("15");
    await runFor(30, (signal) => r.sdk.inbox.serve("agent-1", { handler, pollIntervalMs: 30_000, signal }));
    const inbox = r.pulls.filter((p) => p.path === "/v1/inbox");
    expect(r.pulls[0]!.path).toBe("/v1/inbox/notices");
    expect(Number(inbox[0]!.query.wait)).toBeLessThanOrEqual(15);
    expect(Number(inbox[0]!.query.wait)).toBeGreaterThanOrEqual(1);
  });

  it("inbox.serve sleeps between empty pulls from a server that doesn't wait", async () => {
    for (const header of [undefined, "0"]) {
      const r = emptyRelay(header);
      const took = await runFor(60, (signal) =>
        r.sdk.inbox.serve("agent-1", { handler, noticeIntervalMs: 0, pollIntervalMs: 30_000, signal }),
      );
      expect(took).toBeLessThan(1000);
      expect(r.pulls.length).toBe(1);
    }
  });

  it("waitSecs: 0 polls as before", async () => {
    const r = emptyRelay();
    await runFor(30, (signal) =>
      r.sdk.inbox.serve("agent-1", { handler, noticeIntervalMs: 0, waitSecs: 0, pollIntervalMs: 30_000, signal }),
    );
    expect(r.pulls[0]!.query).toEqual({ agent_id: "agent-1", limit: "25" });
  });

  it("calls.serve pulls again at once while the relay waits, and sleeps otherwise", async () => {
    const waiting = emptyRelay("25");
    await runFor(60, (signal) =>
      waiting.sdk.calls.serve("agent-1", { onMessage: () => undefined, pollIntervalMs: 30_000, signal }),
    );
    expect(waiting.pulls.length).toBeGreaterThan(3);
    expect(waiting.pulls[0]!.query).toEqual({ agent_id: "agent-1", limit: "25", wait: "25" });

    const old = emptyRelay();
    await runFor(60, (signal) =>
      old.sdk.calls.serve("agent-1", { onMessage: () => undefined, pollIntervalMs: 30_000, signal }),
    );
    expect(old.pulls.length).toBe(1);
  });

  it("aborting stops a serve loop whose pull is held open", async () => {
    let started = 0;
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      // A relay that holds every pull until the request is aborted.
      fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
        started += 1;
        const signal = new Request(input, init).signal;
        return await new Promise<Response>((_, reject) => {
          signal.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        });
      }) as typeof fetch,
    });
    const errors: unknown[] = [];
    const took = await runFor(30, (signal) =>
      sdk.inbox.serve("agent-1", {
        handler,
        noticeIntervalMs: 0,
        onError: (err) => errors.push(err),
        signal,
      }),
    );
    expect(took).toBeLessThan(1000);
    expect(started).toBe(1);
    expect(errors).toEqual([]);
  });
});
