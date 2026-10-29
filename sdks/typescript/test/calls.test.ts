/**
 * Calls, hermetic: a `fetch` stub stands in for the relay, as in
 * smoke.test.ts.
 */
import { describe, expect, it, vi } from "vitest";
import {
  ChakraMCP,
  ChakraMCPError,
  type Call,
  type CallEntry,
  type CallFeedItem,
  type CallServeOptions,
} from "../src/index.js";

function mockFetch(handler: (req: Request) => Promise<Response> | Response): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input, init);
    return await handler(req);
  }) as typeof fetch;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const CALLER = "11111111-1111-1111-1111-111111111111";
const CALLEE = "22222222-2222-2222-2222-222222222222";

function callJson(over: Partial<Call> = {}): Call {
  return {
    id: "call-1",
    state: "ringing",
    caller: { agent_id: CALLER, account_id: "acc-1", display_name: "Alice Bot", account_slug: "alice" },
    callee: { agent_id: CALLEE, account_id: "acc-2", display_name: "Bob Bot", account_slug: "bob" },
    memory: "call",
    memory_features: { tags: true, recall: true },
    ttl_secs: 2_592_000,
    idle_secs: 86_400,
    topic: "Venue for the offsite",
    created_at: "2026-10-26T10:00:00Z",
    answered_at: null,
    ended_at: null,
    ended_by: null,
    purge_at: null,
    purged_at: null,
    message_count: 0,
    last_message_at: null,
    ...over,
  };
}

function entryJson(over: Partial<CallEntry> = {}): CallEntry {
  return {
    seq: 1,
    kind: "message",
    from_agent_id: CALLER,
    text: "hello",
    data: null,
    blocked: false,
    created_at: "2026-10-26T10:00:01Z",
    ...over,
  };
}

interface Seen {
  method: string;
  path: string;
  query: Record<string, string>;
  body: unknown;
}

/** An SDK whose relay answers through `route`, recording every request. */
function rig(route: (req: Request, url: URL) => Response | Promise<Response>): {
  sdk: ChakraMCP;
  seen: Seen[];
} {
  const seen: Seen[] = [];
  const sdk = new ChakraMCP({
    apiKey: "ck_test",
    relayUrl: "http://relay",
    fetch: mockFetch(async (req) => {
      const url = new URL(req.url);
      const text = await req.clone().text();
      seen.push({
        method: req.method,
        path: url.pathname,
        query: Object.fromEntries(url.searchParams),
        body: text ? JSON.parse(text) : undefined,
      });
      return route(req, url);
    }),
  });
  return { sdk, seen };
}

describe("CallsClient", () => {
  it("start sends the body and parses a Call", async () => {
    const { sdk, seen } = rig(() => json(callJson(), 201));
    const call = await sdk.calls.start({
      from_agent_id: CALLER,
      to_agent_id: CALLEE,
      memory: "call",
      ttl_secs: 604_800,
      idle_secs: 3_600,
      topic: "Venue for the offsite",
      message: "Shall we book Hall A?",
    });
    expect(seen).toEqual([
      {
        method: "POST",
        path: "/v1/calls",
        query: {},
        body: {
          from_agent_id: CALLER,
          to_agent_id: CALLEE,
          memory: "call",
          ttl_secs: 604_800,
          idle_secs: 3_600,
          topic: "Venue for the offsite",
          message: "Shall we book Hall A?",
        },
      },
    ]);
    expect(call.id).toBe("call-1");
    expect(call.state).toBe("ringing");
    expect(call.caller.agent_id).toBe(CALLER);
    expect(call.memory_features).toEqual({ tags: true, recall: true });
  });

  it("list passes the query and reads next_cursor", async () => {
    const { sdk, seen } = rig(() =>
      json({ calls: [callJson({ state: "active" })], next_cursor: "cur-2" }),
    );
    const page = await sdk.calls.list({ agent_id: CALLER, state: "active", limit: 1, cursor: "cur-1" });
    expect(seen[0]).toMatchObject({
      method: "GET",
      path: "/v1/calls",
      query: { agent_id: CALLER, state: "active", limit: "1", cursor: "cur-1" },
    });
    expect(page.calls.map((c) => c.state)).toEqual(["active"]);
    expect(page.next_cursor).toBe("cur-2");

    // No query: all the user's agents' calls, and no cursor on the last page.
    const { sdk: sdk2, seen: seen2 } = rig(() => json({ calls: [] }));
    const last = await sdk2.calls.list();
    expect(seen2[0]).toMatchObject({ method: "GET", path: "/v1/calls", query: {} });
    expect(last.next_cursor).toBeUndefined();
  });

  it("send and history round trip", async () => {
    const stored: CallEntry[] = [];
    const { sdk, seen } = rig(async (req, url) => {
      if (req.method === "POST" && url.pathname === "/v1/calls/call-1/messages") {
        const body = (await req.json()) as { text: string; data?: unknown };
        const entry = entryJson({ seq: stored.length + 2, text: body.text, data: body.data ?? null });
        stored.push(entry);
        return json(entry, 201);
      }
      if (req.method === "GET" && url.pathname === "/v1/calls/call-1/messages") {
        const after = Number(url.searchParams.get("after") ?? 0);
        return json(stored.filter((e) => e.seq > after));
      }
      return json({ error: { code: "not_found", message: "not found" } }, 404);
    });

    const sent = await sdk.calls.send("call-1", {
      agent_id: CALLER,
      text: "Hall A, Friday 10:00?",
      data: { hall: "A" },
    });
    expect(sent).toMatchObject({ seq: 2, kind: "message", text: "Hall A, Friday 10:00?", data: { hall: "A" } });
    expect(seen[0]!.body).toEqual({ agent_id: CALLER, text: "Hall A, Friday 10:00?", data: { hall: "A" } });

    const history = await sdk.calls.history("call-1", { after: 1, limit: 50, q: "hall friday" });
    expect(seen[1]).toMatchObject({
      method: "GET",
      path: "/v1/calls/call-1/messages",
      query: { after: "1", limit: "50", q: "hall friday" },
    });
    expect(history).toEqual([sent]);
  });

  it("ask parses a recall", async () => {
    const { sdk, seen } = rig(() =>
      json({
        answered: 0.93,
        messages: [
          { seq: 4, from: CALLEE, text: "Hall A, agreed.", at: "2026-10-26T10:05:00Z", p: 0.88 },
        ],
      }),
    );
    const recall = await sdk.calls.ask("call-1", { agent_id: CALLER, question: "Which hall?" });
    expect(seen[0]).toMatchObject({
      method: "POST",
      path: "/v1/calls/call-1/memory/ask",
      body: { agent_id: CALLER, question: "Which hall?" },
    });
    expect(recall.answered).toBe(0.93);
    expect(recall.messages[0]).toMatchObject({ seq: 4, text: "Hall A, agreed.", p: 0.88 });
  });

  it("an error with a chk.calls.* code surfaces it", async () => {
    const { sdk } = rig(() =>
      json({ error: { code: "chk.calls.busy", message: "Bob Bot is on too many calls" } }, 409),
    );
    const err = await sdk.calls.start({ from_agent_id: CALLER, to_agent_id: CALLEE }).catch((e) => e);
    expect(err).toBeInstanceOf(ChakraMCPError);
    expect(err).toMatchObject({
      status: 409,
      code: "chk.calls.busy",
      message: "Bob Bot is on too many calls",
    });

    const { sdk: sdk2 } = rig(() =>
      json({ error: { code: "chk.calls.memory_unavailable", message: "no recall" } }, 503),
    );
    await expect(
      sdk2.calls.ask("call-1", { agent_id: CALLER, question: "?" }),
    ).rejects.toMatchObject({ status: 503, code: "chk.calls.memory_unavailable" });
  });

  it("the other routes hit their paths", async () => {
    const { sdk, seen } = rig((_req, url) => {
      if (url.pathname === "/v1/calls/call%201/memory") return json({ pinned: [], open: [], recent: [] });
      if (url.pathname === "/v1/inbox/calls") return json([]);
      return json(callJson());
    });
    await sdk.calls.get("call 1");
    await sdk.calls.accept("call 1", { agent_id: CALLEE, message: "Hi!" });
    await sdk.calls.decline("call 1", { agent_id: CALLEE, reason: "busy" });
    await sdk.calls.end("call 1", { agent_id: CALLER });
    const memory = await sdk.calls.memory("call 1");
    await sdk.calls.pull(CALLEE, { limit: 10 });
    expect(seen.map((s) => [s.method, s.path, s.body])).toEqual([
      ["GET", "/v1/calls/call%201", undefined],
      ["POST", "/v1/calls/call%201/accept", { agent_id: CALLEE, message: "Hi!" }],
      ["POST", "/v1/calls/call%201/decline", { agent_id: CALLEE, reason: "busy" }],
      ["POST", "/v1/calls/call%201/end", { agent_id: CALLER }],
      ["GET", "/v1/calls/call%201/memory", undefined],
      ["GET", "/v1/inbox/calls", undefined],
    ]);
    expect(seen[5]!.query).toEqual({ agent_id: CALLEE, limit: "10" });
    expect(memory).toEqual({ pinned: [], open: [], recent: [] });
  });

  it("invoke sends call_id", async () => {
    const { sdk, seen } = rig(() => json({ invocation_id: "inv-1", status: "pending", error: null }));
    await sdk.invoke({ grant_id: "grant-1", grantee_agent_id: CALLER, input: { q: 1 }, call_id: "call-1" });
    await sdk.invoke({ capability_id: "cap-1", grantee_agent_id: CALLER, input: {}, call_id: "call-1" });
    expect(seen.map((s) => s.body)).toEqual([
      { grant_id: "grant-1", grantee_agent_id: CALLER, input: { q: 1 }, call_id: "call-1" },
      { capability_id: "cap-1", grantee_agent_id: CALLER, input: {}, call_id: "call-1" },
    ]);
  });
});

// ─── calls.serve ─────────────────────────────────────────

interface ServeRig {
  sdk: ChakraMCP;
  seen: Seen[];
  pulls: () => number;
}

/**
 * The feed hands out `batches` one per pull, then empty pulls. `sends`
 * answers replies (default: 201 with an entry).
 */
function serveRig(
  batches: CallFeedItem[][],
  sends: (callId: string) => Response = () => json(entryJson({ from_agent_id: CALLEE }), 201),
): ServeRig {
  let pulled = 0;
  const { sdk, seen } = rig((req, url) => {
    if (url.pathname === "/v1/inbox/calls") {
      const batch = batches[pulled] ?? [];
      pulled += 1;
      return json(batch);
    }
    const m = url.pathname.match(/^\/v1\/calls\/([^/]+)\/(accept|decline|messages)$/);
    if (req.method === "POST" && m) {
      if (m[2] === "messages") return sends(m[1]!);
      return json(callJson({ id: m[1]!, state: m[2] === "accept" ? "active" : "declined" }));
    }
    return json({ error: { code: "not_found", message: "not found" } }, 404);
  });
  return { sdk, seen, pulls: () => pulled };
}

/** Run `serve` until the feed has been drained, then cancel it. */
async function serveOnce(r: ServeRig, opts: Omit<CallServeOptions, "signal" | "pollIntervalMs">): Promise<void> {
  const ac = new AbortController();
  const p = r.sdk.calls.serve(CALLEE, { ...opts, pollIntervalMs: 5, signal: ac.signal });
  await new Promise((resolve) => setTimeout(resolve, 50));
  ac.abort();
  await p;
}

function item(callId: string, entry: Partial<CallEntry>, over: Partial<CallFeedItem> = {}): CallFeedItem {
  return { call: callJson({ id: callId, state: "active" }), entry: entryJson(entry), ...over };
}

/** The relay calls the serve loop made, past pulling the feed. */
function actions(seen: Seen[]): Array<[string, unknown]> {
  return seen.filter((s) => s.path !== "/v1/inbox/calls").map((s) => [s.path, s.body]);
}

describe("CallsClient.serve", () => {
  it("accepts an invite by default and replies to a message through onMessage", async () => {
    const memory = { pinned: [], open: [], recent: [] };
    const r = serveRig([
      [
        item("call-1", { seq: 1, kind: "invite", text: "Shall we book Hall A?" }, {
          call: callJson({ id: "call-1", state: "ringing" }),
        }),
        item("call-2", { seq: 3, kind: "message", text: "Friday?" }, { memory }),
        item("call-3", { seq: 5, kind: "message", text: "Thanks" }),
      ],
    ]);
    const got: CallFeedItem[] = [];
    await serveOnce(r, {
      onMessage: (it) => {
        got.push(it);
        if (it.entry.text === "Friday?") return { text: "Friday works", data: { day: "fri" } };
        return undefined; // no reply
      },
    });
    expect(actions(r.seen)).toEqual([
      ["/v1/calls/call-1/accept", { agent_id: CALLEE }],
      ["/v1/calls/call-2/messages", { agent_id: CALLEE, text: "Friday works", data: { day: "fri" } }],
    ]);
    expect(got.map((it) => it.call.id)).toEqual(["call-2", "call-3"]);
    expect(got[0]!.memory).toEqual(memory);
    // Each pull asks the feed for the agent, at most 25 at a time, and
    // lets the relay wait up to 25 seconds for events.
    expect(r.seen[0]!.query).toEqual({ agent_id: CALLEE, limit: "25", wait: "25" });
  });

  it("sends a text reply as is, and onInvite can accept with a reply or decline", async () => {
    const ringing = { call: callJson({ id: "call-1", state: "ringing" }) };
    const r = serveRig([
      [
        item("call-1", { seq: 1, kind: "invite" }, ringing),
        item("call-2", { seq: 1, kind: "invite" }, { call: callJson({ id: "call-2", state: "ringing" }) }),
        item("call-3", { seq: 4, kind: "message", text: "ping" }),
      ],
    ]);
    await serveOnce(r, {
      onInvite: ({ call }) =>
        call.id === "call-1" ? { accept: true, message: "Hello!" } : { accept: false, reason: "busy" },
      onMessage: async ({ entry }) => `pong to ${entry.text}`,
    });
    expect(actions(r.seen)).toEqual([
      ["/v1/calls/call-1/accept", { agent_id: CALLEE, message: "Hello!" }],
      ["/v1/calls/call-2/decline", { agent_id: CALLEE, reason: "busy" }],
      ["/v1/calls/call-3/messages", { agent_id: CALLEE, text: "pong to ping" }],
    ]);
  });

  it("calls onEvent for an end, and ignores events without it", async () => {
    const ended = item("call-1", { seq: 9, kind: "ended", text: null, from_agent_id: CALLER }, {
      call: callJson({ id: "call-1", state: "ended", ended_by: "caller" }),
    });
    const r = serveRig([[ended]]);
    const events: string[] = [];
    const onMessage = vi.fn();
    await serveOnce(r, {
      onMessage,
      onEvent: ({ call, entry }) => {
        events.push(`${entry.kind} ${call.ended_by}`);
      },
    });
    expect(events).toEqual(["ended caller"]);
    expect(onMessage).not.toHaveBeenCalled();
    expect(actions(r.seen)).toEqual([]);

    const r2 = serveRig([[ended]]);
    await serveOnce(r2, { onMessage });
    expect(actions(r2.seen)).toEqual([]);
  });

  it("survives a handler error and carries on with the next item", async () => {
    const r = serveRig([
      [
        item("call-1", { seq: 2, kind: "message", text: "boom" }),
        item("call-1", { seq: 3, kind: "message", text: "fine" }),
      ],
      [item("call-1", { seq: 4, kind: "message", text: "later" })],
    ]);
    const errors: Array<{ msg: string; seq?: number }> = [];
    await serveOnce(r, {
      onMessage: ({ entry }) => {
        if (entry.text === "boom") throw new Error("model down");
        return `ok ${entry.seq}`;
      },
      onError: (err, it) => {
        errors.push({ msg: err instanceof Error ? err.message : String(err), seq: it?.entry.seq });
      },
    });
    expect(errors).toEqual([{ msg: "model down", seq: 2 }]);
    expect(actions(r.seen)).toEqual([
      ["/v1/calls/call-1/messages", { agent_id: CALLEE, text: "ok 3" }],
      ["/v1/calls/call-1/messages", { agent_id: CALLEE, text: "ok 4" }],
    ]);
  });

  it("logs a handler error when there's no onError", async () => {
    const r = serveRig([[item("call-1", { seq: 2, kind: "message" })]]);
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    await serveOnce(r, {
      onMessage: () => {
        throw new Error("model down");
      },
    });
    expect(errorSpy).toHaveBeenCalledTimes(1);
    expect(String(errorSpy.mock.calls[0]![0])).toContain("call call-1, entry 2");
    errorSpy.mockRestore();
  });

  it("drops a reply to a call that ended meanwhile, but reports other send errors", async () => {
    const r = serveRig(
      [
        [
          item("gone", { seq: 2, kind: "message" }, { call: callJson({ id: "gone", state: "active" }) }),
          item("slow", { seq: 2, kind: "message" }, { call: callJson({ id: "slow", state: "active" }) }),
        ],
      ],
      (callId) =>
        callId === "gone"
          ? json({ error: { code: "chk.calls.not_active", message: "this call is ended" } }, 409)
          : json({ error: { code: "chk.calls.slow_down", message: "slow down" } }, 429),
    );
    const errors: unknown[] = [];
    await serveOnce(r, { onMessage: () => "reply", onError: (err) => errors.push(err) });
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatchObject({ status: 429, code: "chk.calls.slow_down" });
  });

  it("stops when cancelled", async () => {
    const r = serveRig([]);
    const ac = new AbortController();
    const p = r.sdk.calls.serve(CALLEE, { onMessage: () => undefined, pollIntervalMs: 5, signal: ac.signal });
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(r.pulls()).toBeGreaterThan(1);
    ac.abort();
    await p;
    const after = r.pulls();
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(r.pulls()).toBe(after);

    // Already cancelled: it never pulls.
    const r2 = serveRig([]);
    await r2.sdk.calls.serve(CALLEE, { onMessage: () => undefined, signal: AbortSignal.abort() });
    expect(r2.pulls()).toBe(0);
  });

  it("keeps going when a pull fails", async () => {
    let pulled = 0;
    const { sdk, seen } = rig((_req, url) => {
      if (url.pathname === "/v1/inbox/calls") {
        pulled += 1;
        if (pulled === 1) return json({ error: { code: "unavailable", message: "try later" } }, 503);
        if (pulled === 2) return json([item("call-1", { seq: 2, kind: "message", text: "hi" })]);
        return json([]);
      }
      return json(entryJson(), 201);
    });
    const errors: unknown[] = [];
    await serveOnce(
      { sdk, seen, pulls: () => pulled },
      { onMessage: () => "hello", onError: (err) => errors.push(err) },
    );
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatchObject({ status: 503, code: "unavailable" });
    expect(actions(seen)).toEqual([["/v1/calls/call-1/messages", { agent_id: CALLEE, text: "hello" }]]);
  });
});
