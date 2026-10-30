/**
 * Hermetic smoke - a `fetch` stub stands in for the real backend so the
 * test suite runs in CI without a server. Validates that the SDK shapes
 * requests + headers correctly and surfaces error envelopes.
 */
import { describe, expect, it, vi } from "vitest";
import {
  ChakraMCP,
  ChakraMCPError,
  CreditsExhaustedError,
  QuotaExhaustedError,
  RateLimitedError,
  ServeOptions,
} from "../src/index.js";

function mockFetch(handler: (req: Request) => Promise<Response> | Response): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input, init);
    return await handler(req);
  }) as typeof fetch;
}

describe("ChakraMCP", () => {
  it("rejects non-`ck_` API keys at construction", () => {
    expect(() => new ChakraMCP({ apiKey: "not-an-api-key" })).toThrow(/ck_/);
  });

  it("sets Bearer auth on requests + parses success", async () => {
    const seen: string[] = [];
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      appUrl: "http://app",
      relayUrl: "http://relay",
      fetch: mockFetch(async (req) => {
        seen.push(`${req.method} ${req.url}`);
        expect(req.headers.get("authorization")).toBe("Bearer ck_test");
        return new Response(
          JSON.stringify({
            user: { id: "u1", email: "alice@example.com", display_name: "Alice", avatar_url: null, is_admin: false },
            memberships: [],
            survey_required: false,
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }),
    });
    const me = await sdk.me();
    expect(me.user.email).toBe("alice@example.com");
    expect(seen).toEqual(["GET http://app/v1/me"]);
  });

  it("decodes error envelopes into ChakraMCPError", async () => {
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: mockFetch(
        async () =>
          new Response(
            JSON.stringify({ error: { code: "forbidden", message: "forbidden" } }),
            { status: 403, headers: { "content-type": "application/json" } },
          ),
      ),
    });
    await expect(sdk.agents.list()).rejects.toMatchObject({
      name: "ChakraMCPError",
      status: 403,
      code: "forbidden",
    } satisfies Partial<ChakraMCPError>);
  });

  it("network sends limit + cursor and decodes a page", async () => {
    const seen: string[] = [];
    const agent = {
      id: "a1",
      account_id: "acc1",
      account_slug: "alice",
      account_display_name: "Alice",
      slug: "scheduler",
      display_name: "Scheduler",
      description: "Books meetings",
      mode: "pull",
      accept_calls: "friends",
      tags: ["calendar"],
      verified: true,
      endpoint_url: null,
      created_at: "2026-10-26T10:00:00Z",
      updated_at: "2026-10-26T10:00:00Z",
      is_mine: false,
      capability_count: 2,
      avg_rating: 4.5,
      review_count: 3,
      online: true,
    };
    let body: unknown = { agents: [agent], next_cursor: "cur-2" };
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: mockFetch(async (req) => {
        seen.push(`${req.method} ${req.url}`);
        return new Response(JSON.stringify(body), { status: 200 });
      }),
    });

    const page = await sdk.network({ limit: 1, cursor: "cur 1" });
    expect(seen[0]).toBe("GET http://relay/v1/network/agents?limit=1&cursor=cur+1");
    expect(page.next_cursor).toBe("cur-2");
    expect(page.agents).toHaveLength(1);
    expect(page.agents[0]).toMatchObject({
      slug: "scheduler",
      mode: "pull",
      tags: ["calendar"],
      verified: true,
      capability_count: 2,
      online: true,
    });

    // No query; the last page has no next_cursor.
    body = { agents: [] };
    const last = await sdk.network();
    expect(seen[1]).toBe("GET http://relay/v1/network/agents");
    expect(last.agents).toEqual([]);
    expect(last.next_cursor).toBeUndefined();
  });

  it("capabilities.update PATCHes the capability on the relay", async () => {
    const seen: { method: string; url: string; body: unknown }[] = [];
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: mockFetch(async (req) => {
        seen.push({ method: req.method, url: req.url, body: await req.json() });
        expect(req.headers.get("content-type")).toBe("application/json");
        return new Response(
          JSON.stringify({ id: "cap 1", name: "echo", description: "now louder", visibility: "network" }),
          { status: 200, headers: { "content-type": "application/json" } },
        );
      }),
    });
    const cap = await sdk.agents.capabilities.update("agent-1", "cap 1", {
      description: "now louder",
      visibility: "network",
    });
    expect(cap.description).toBe("now louder");
    expect(seen).toEqual([
      {
        method: "PATCH",
        url: "http://relay/v1/agents/agent-1/capabilities/cap%201",
        body: { description: "now louder", visibility: "network" },
      },
    ]);
  });

  it("invoke sends capability_id for a public call, grant_id for a friend call, and consent_terms", async () => {
    const bodies: unknown[] = [];
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: mockFetch(async (req) => {
        bodies.push(await req.json());
        return new Response(JSON.stringify({ invocation_id: "inv1", status: "pending", error: null }), {
          status: 200,
        });
      }),
    });
    await sdk.invoke({ capability_id: "cap-1", grantee_agent_id: "me", input: { q: 1 } });
    await sdk.invoke({ grant_id: "grant-1", grantee_agent_id: "me", input: {} });
    await sdk.invoke({
      grant_id: "grant-1",
      grantee_agent_id: "me",
      input: {},
      consent_terms: { purpose: "book a slot", valid_for: "1d" },
    });
    expect(bodies).toEqual([
      { capability_id: "cap-1", grantee_agent_id: "me", input: { q: 1 } },
      { grant_id: "grant-1", grantee_agent_id: "me", input: {} },
      {
        grant_id: "grant-1",
        grantee_agent_id: "me",
        input: {},
        consent_terms: { purpose: "book a slot", valid_for: "1d" },
      },
    ]);
  });

  it("invokeAndWait polls until terminal", async () => {
    let calls = 0;
    const fetchStub = vi.fn(
      mockFetch(async (req) => {
        const url = new URL(req.url);
        if (req.method === "POST" && url.pathname === "/v1/invoke") {
          return new Response(
            JSON.stringify({ invocation_id: "inv1", status: "pending", error: null }),
            { status: 200 },
          );
        }
        if (req.method === "GET" && url.pathname === "/v1/invocations/inv1") {
          calls += 1;
          const status = calls < 2 ? "in_progress" : "succeeded";
          return new Response(
            JSON.stringify({
              id: "inv1",
              grant_id: "g1",
              granter_agent_id: "a1",
              granter_display_name: "Alice Bot",
              grantee_agent_id: "a2",
              grantee_display_name: "Bob Bot",
              capability_id: "c1",
              capability_name: "echo",
              status,
              elapsed_ms: 100,
              error_message: null,
              input_preview: { hello: "world" },
              output_preview: status === "succeeded" ? { echoed: "world" } : null,
              created_at: "2026-01-01T00:00:00Z",
              claimed_at: null,
              i_served: false,
              i_invoked: true,
            }),
            { status: 200 },
          );
        }
        return new Response("not found", { status: 404 });
      }),
    );

    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });

    const final = await sdk.invokeAndWait(
      { grant_id: "g1", grantee_agent_id: "a2", input: { hello: "world" } },
      { intervalMs: 5, timeoutMs: 5000 },
    );
    expect(final.status).toBe("succeeded");
    expect(final.output_preview).toEqual({ echoed: "world" });
  });

  it("inbox.serve loops, dispatches handler, reports results, exits on abort", async () => {
    let pulled = 0;
    const reported: Array<{ id: string; status: string }> = [];
    const fetchStub = mockFetch(async (req) => {
      const url = new URL(req.url);
      if (url.pathname === "/v1/inbox") {
        pulled += 1;
        if (pulled > 1) {
          return new Response(JSON.stringify([]), { status: 200 });
        }
        return new Response(
          JSON.stringify([
            {
              id: "inv1",
              grant_id: null,
              granter_agent_id: null,
              granter_display_name: null,
              grantee_agent_id: null,
              grantee_display_name: null,
              capability_id: null,
              capability_name: "echo",
              status: "in_progress",
              elapsed_ms: 0,
              error_message: null,
              input_preview: { hello: "world" },
              output_preview: null,
              created_at: "2026-01-01T00:00:00Z",
              claimed_at: "2026-01-01T00:00:01Z",
              i_served: true,
              i_invoked: false,
            },
          ]),
          { status: 200 },
        );
      }
      if (req.method === "POST" && url.pathname.endsWith("/result")) {
        const body = JSON.parse(await req.text());
        reported.push({ id: url.pathname.split("/")[3]!, status: body.status });
        return new Response(JSON.stringify({}), { status: 200 });
      }
      return new Response("not found", { status: 404 });
    });
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });
    const ac = new AbortController();
    const handlerSeen: string[] = [];
    // Exercise the legacy positional `(agentId, handler, opts)` form;
    // the new options-object form is covered in the HITL routing tests
    // below.
    const servePromise = sdk.inbox.serve(
      "agent-id",
      async (inv) => {
        handlerSeen.push(inv.id);
        return { status: "succeeded", output: { ok: true } };
      },
      { pollIntervalMs: 5, signal: ac.signal },
    );
    // Wait long enough for one batch to be processed.
    await new Promise((r) => setTimeout(r, 50));
    ac.abort();
    await servePromise;
    expect(handlerSeen).toEqual(["inv1"]);
    expect(reported).toEqual([{ id: "inv1", status: "succeeded" }]);
  });
});

// ─── HITL routing ────────────────────────────────────────
//
// On a `human_in_loop` capability a person stands behind each reply: an
// agent's reply is a draft until one of its owners confirms it on the web.
// `serve()` sends these invocations to `humanHandler`, which either
// returns a proposed reply (posted, so it becomes a draft) or nothing
// (the owner answers in the web app's inbox).

interface ServeRig {
  reported: Array<{ id: string; body: Record<string, unknown> }>;
  fetchStub: typeof fetch;
}

function buildServeRig(invocations: Array<Partial<Record<string, unknown>>>): ServeRig {
  const reported: Array<{ id: string; body: Record<string, unknown> }> = [];
  let pulled = 0;
  const fetchStub = mockFetch(async (req) => {
    const url = new URL(req.url);
    if (url.pathname === "/v1/inbox/notices") {
      return new Response(JSON.stringify([]), { status: 200 });
    }
    if (url.pathname === "/v1/inbox") {
      pulled += 1;
      if (pulled > 1) {
        return new Response(JSON.stringify([]), { status: 200 });
      }
      const baseline = {
        grant_id: null,
        granter_agent_id: null,
        granter_display_name: null,
        grantee_agent_id: null,
        grantee_display_name: null,
        capability_id: null,
        capability_name: "echo",
        status: "in_progress",
        elapsed_ms: 0,
        error_message: null,
        input_preview: { hello: "world" },
        output_preview: null,
        created_at: "2026-01-01T00:00:00Z",
        claimed_at: "2026-01-01T00:00:01Z",
        i_served: true,
        i_invoked: false,
      };
      return new Response(
        JSON.stringify(invocations.map((inv) => ({ ...baseline, ...inv }))),
        { status: 200 },
      );
    }
    if (req.method === "POST" && url.pathname.endsWith("/result")) {
      const body = JSON.parse(await req.text());
      const id = url.pathname.split("/")[3]!;
      reported.push({ id, body });
      const hitl = invocations.some((i) => i.id === id && i.semantics === "human_in_loop");
      if (hitl) {
        return new Response(
          JSON.stringify({
            status: "awaiting_confirmation",
            consent_request_id: "cr-1",
            approve_url: "https://chakramcp.com/app/approvals/cr-1",
            expires_at: "2026-01-02T00:00:00Z",
          }),
          { status: 202 },
        );
      }
      return new Response(JSON.stringify({}), { status: 200 });
    }
    return new Response("not found", { status: 404 });
  });
  return { reported, fetchStub };
}

async function runOneBatch(
  sdk: ChakraMCP,
  opts: Omit<ServeOptions, "signal">,
): Promise<void> {
  const ac = new AbortController();
  const p = sdk.inbox.serve("agent-id", { ...opts, signal: ac.signal });
  await new Promise((r) => setTimeout(r, 50));
  ac.abort();
  await p;
}

describe("InboxClient.serve HITL routing", () => {
  it("without a humanHandler, warns with the owner's reply link", async () => {
    const link = "https://chakramcp.com/app/messages/inv-hitl";
    const { reported, fetchStub } = buildServeRig([
      {
        id: "inv-hitl",
        capability_name: "message_owner",
        semantics: "human_in_loop",
        owner_reply_url: link,
      },
    ]);
    const sdk = new ChakraMCP({ apiKey: "ck_test", relayUrl: "http://relay", fetch: fetchStub });
    const warned: string[] = [];
    const warn = vi.spyOn(console, "warn").mockImplementation((m: string) => {
      warned.push(m);
    });
    try {
      await runOneBatch(sdk, {
        handler: async () => ({ status: "succeeded", output: {} }),
        pollIntervalMs: 5,
      });
    } finally {
      warn.mockRestore();
    }
    expect(reported).toEqual([]);
    expect(warned.some((m) => m.includes("inv-hitl") && m.endsWith(link))).toBe(true);
  });

  it("posts a reply humanHandler returns, which becomes a draft", async () => {
    const { reported, fetchStub } = buildServeRig([
      { id: "inv-hitl", capability_name: "message_owner", semantics: "human_in_loop" },
    ]);
    const sdk = new ChakraMCP({ apiKey: "ck_test", relayUrl: "http://relay", fetch: fetchStub });
    const drafts: string[] = [];
    await runOneBatch(sdk, {
      handler: async () => ({ status: "succeeded", output: {} }),
      humanHandler: async () => ({ status: "succeeded", output: { reply: "Friday works" } }),
      onDraft: (inv, drafted) => {
        drafts.push(`${inv.id} ${drafted.approve_url}`);
      },
      pollIntervalMs: 5,
    });
    expect(reported).toEqual([
      { id: "inv-hitl", body: { status: "succeeded", output: { reply: "Friday works" } } },
    ]);
    expect(drafts).toEqual(["inv-hitl https://chakramcp.com/app/approvals/cr-1"]);
  });

  it("routes human_in_loop to humanHandler and does NOT post a result", async () => {
    const { reported, fetchStub } = buildServeRig([
      {
        id: "inv-hitl",
        capability_name: "message_owner",
        semantics: "human_in_loop",
      },
    ]);
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });

    const autoSeen: string[] = [];
    const humanSeen: string[] = [];
    await runOneBatch(sdk, {
      handler: async (inv) => {
        autoSeen.push(inv.id);
        return { status: "succeeded", output: {} };
      },
      humanHandler: async (inv) => {
        humanSeen.push(inv.id);
      },
      pollIntervalMs: 5,
    });

    expect(humanSeen).toEqual(["inv-hitl"]);
    expect(autoSeen).toEqual([]);
    expect(reported).toEqual([]);
  });

  it("warns + does NOT post a result when humanHandler is missing", async () => {
    const { reported, fetchStub } = buildServeRig([
      {
        id: "inv-hitl-warn",
        capability_name: "message_owner",
        semantics: "human_in_loop",
      },
    ]);
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });

    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    const autoSeen: string[] = [];
    await runOneBatch(sdk, {
      handler: async (inv) => {
        autoSeen.push(inv.id);
        return { status: "succeeded", output: {} };
      },
      pollIntervalMs: 5,
    });

    expect(autoSeen).toEqual([]);
    expect(reported).toEqual([]);
    expect(warnSpy).toHaveBeenCalledTimes(1);
    const warnMsg = String(warnSpy.mock.calls[0]![0]);
    expect(warnMsg).toContain("inv-hitl-warn");
    expect(warnMsg).toContain("human_in_loop");
    expect(warnMsg).toContain("web app's inbox");
    warnSpy.mockRestore();
  });

  it("falls through to handler for autonomous (regression)", async () => {
    const { reported, fetchStub } = buildServeRig([
      {
        id: "inv-auto",
        capability_name: "echo",
        semantics: "autonomous",
      },
    ]);
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });

    const humanSeen: string[] = [];
    const autoSeen: string[] = [];
    await runOneBatch(sdk, {
      handler: async (inv) => {
        autoSeen.push(inv.id);
        return { status: "succeeded", output: { ok: true } };
      },
      humanHandler: async (inv) => {
        humanSeen.push(inv.id);
      },
      pollIntervalMs: 5,
    });

    expect(autoSeen).toEqual(["inv-auto"]);
    expect(humanSeen).toEqual([]);
    expect(reported).toEqual([
      { id: "inv-auto", body: { status: "succeeded", output: { ok: true } } },
    ]);
  });

  it("treats invocations missing semantics as autonomous (forward-compat with relay pre-DTO-plumbing)", async () => {
    // Pre-PR-5 relay builds don't populate Invocation.semantics yet —
    // the SDK should still happily run the autonomous handler so
    // existing workers don't silently stall.
    const { reported, fetchStub } = buildServeRig([
      {
        id: "inv-no-semantics",
        capability_name: "echo",
      },
    ]);
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });

    const autoSeen: string[] = [];
    await runOneBatch(sdk, {
      handler: async (inv) => {
        autoSeen.push(inv.id);
        return { status: "succeeded", output: { ok: true } };
      },
      pollIntervalMs: 5,
    });

    expect(autoSeen).toEqual(["inv-no-semantics"]);
    expect(reported).toEqual([
      { id: "inv-no-semantics", body: { status: "succeeded", output: { ok: true } } },
    ]);
  });

  it("surfaces humanHandler throws via onError without posting a result", async () => {
    const { reported, fetchStub } = buildServeRig([
      {
        id: "inv-hitl-boom",
        capability_name: "message_owner",
        semantics: "human_in_loop",
      },
    ]);
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: fetchStub,
    });

    const errors: Array<{ msg: string; invId?: string }> = [];
    await runOneBatch(sdk, {
      handler: async () => ({ status: "succeeded", output: {} }),
      humanHandler: async () => {
        throw new Error("slack down");
      },
      onError: (err, inv) => {
        errors.push({
          msg: err instanceof Error ? err.message : String(err),
          invId: inv?.id,
        });
      },
      pollIntervalMs: 5,
    });

    expect(errors).toEqual([{ msg: "slack down", invId: "inv-hitl-boom" }]);
    // CRITICAL: the SDK must NOT report a wire result — a human still
    // has to resolve this invocation.
    expect(reported).toEqual([]);
  });
});

describe("usage-limit errors", () => {
  /** An SDK whose relay answers every call with one canned error. */
  function sdkReturning(
    status: number,
    body: unknown,
    headers: Record<string, string> = {},
  ): ChakraMCP {
    return new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: mockFetch(
        async () =>
          new Response(JSON.stringify(body), {
            status,
            headers: { "content-type": "application/json", ...headers },
          }),
      ),
    });
  }

  /** Run `invoke` and hand back what it threw. */
  async function invokeError(sdk: ChakraMCP): Promise<unknown> {
    try {
      await sdk.invoke({ capability_id: "cap", grantee_agent_id: "agent", input: {} });
    } catch (err) {
      return err;
    }
    throw new Error("expected invoke to throw");
  }

  it("rate limited: RateLimitedError with the Retry-After delay", async () => {
    const err = await invokeError(
      sdkReturning(
        429,
        {
          error: {
            code: "account_rate_limited",
            message: "rate limit exceeded",
            retryable: true,
            retry_after_secs: 30,
          },
        },
        { "retry-after": "23" },
      ),
    );
    expect(err).toBeInstanceOf(RateLimitedError);
    // Still a ChakraMCPError, so existing catch blocks keep working.
    expect(err).toBeInstanceOf(ChakraMCPError);
    expect(err).toMatchObject({
      name: "RateLimitedError",
      status: 429,
      code: "account_rate_limited",
      message: "rate limit exceeded",
      // The header wins over the body.
      retry_after_secs: 23,
    });
  });

  it("credits exhausted on chakramcp.com: next grant + credits link", async () => {
    const err = await invokeError(
      sdkReturning(
        429,
        {
          error: {
            code: "account_credits_exhausted",
            message: "insufficient credits",
            retryable: false,
            retry_after_secs: 1300000,
            next_grant_at: "2026-11-01T00:00:00Z",
            credits_url: "https://chakramcp.com/app/credits",
          },
        },
        { "retry-after": "1300000" },
      ),
    );
    expect(err).toBeInstanceOf(CreditsExhaustedError);
    expect(err).toBeInstanceOf(ChakraMCPError);
    expect(err).toMatchObject({
      name: "CreditsExhaustedError",
      status: 429,
      code: "account_credits_exhausted",
      retry_after_secs: 1300000,
      next_grant_at: "2026-11-01T00:00:00Z",
      credits_url: "https://chakramcp.com/app/credits",
    });
  });

  it("credits exhausted on a self-hosted server: no credits link", async () => {
    const err = await invokeError(
      sdkReturning(
        429,
        {
          error: {
            code: "account_credits_exhausted",
            message: "insufficient credits",
            retryable: false,
            retry_after_secs: 86400,
            next_grant_at: "2026-11-01T00:00:00Z",
          },
        },
        { "retry-after": "86400" },
      ),
    );
    expect(err).toBeInstanceOf(CreditsExhaustedError);
    const credits = err as CreditsExhaustedError;
    expect(credits.retry_after_secs).toBe(86400);
    expect(credits.next_grant_at).toBe("2026-11-01T00:00:00Z");
    expect(credits.credits_url).toBeUndefined();
  });

  it("takes the delay from the body when there's no Retry-After header", async () => {
    const err = await invokeError(
      sdkReturning(429, {
        error: {
          code: "account_rate_limited",
          message: "rate limit exceeded",
          retryable: true,
          retry_after_secs: 17,
        },
      }),
    );
    expect(err).toBeInstanceOf(RateLimitedError);
    expect((err as RateLimitedError).retry_after_secs).toBe(17);
  });

  it("monthly quota: QuotaExhaustedError gains the Retry-After delay", async () => {
    const err = await invokeError(
      sdkReturning(
        429,
        {
          error: {
            code: "monthly_quota_exhausted",
            message: "monthly quota exhausted",
            retryable: false,
          },
          quota: 100,
          resets_at: "2026-11-01T00:00:00Z",
        },
        { "retry-after": "1300000" },
      ),
    );
    expect(err).toBeInstanceOf(QuotaExhaustedError);
    expect(err).toMatchObject({
      code: "monthly_quota_exhausted",
      quota: 100,
      resets_at: "2026-11-01T00:00:00Z",
      retry_after_secs: 1300000,
    });
  });

  it("older servers: typed errors with no delay, grant time or link", async () => {
    const credits = await invokeError(
      sdkReturning(429, {
        error: { code: "account_credits_exhausted", message: "insufficient credits", retryable: false },
      }),
    );
    expect(credits).toBeInstanceOf(CreditsExhaustedError);
    expect(credits).toMatchObject({
      retry_after_secs: undefined,
      next_grant_at: undefined,
      credits_url: undefined,
    });

    const rate = await invokeError(
      sdkReturning(429, {
        error: { code: "account_rate_limited", message: "rate limit exceeded", retryable: true },
      }),
    );
    expect(rate).toBeInstanceOf(RateLimitedError);
    expect((rate as RateLimitedError).retry_after_secs).toBeUndefined();

    const quota = await invokeError(
      sdkReturning(429, {
        error: { code: "monthly_quota_exhausted", message: "monthly quota exhausted" },
        quota: 100,
        resets_at: "2026-11-01T00:00:00Z",
      }),
    );
    expect(quota).toBeInstanceOf(QuotaExhaustedError);
    expect((quota as QuotaExhaustedError).retry_after_secs).toBeUndefined();
  });

  it("a refused invoke carries its reason and code", async () => {
    const err = await invokeError(
      sdkReturning(403, {
        invocation_id: "inv",
        status: "rejected",
        error: "System One safety check blocked this request: threatens violence",
        code: "chk.policy.safety_blocked",
      }),
    );
    expect(err).toBeInstanceOf(ChakraMCPError);
    expect(err).toMatchObject({
      status: 403,
      code: "chk.policy.safety_blocked",
      message: "System One safety check blocked this request: threatens violence",
    });
  });

  it("other errors stay ChakraMCPError, with a delay only when the server sends one", async () => {
    const forbidden = await invokeError(
      sdkReturning(403, { error: { code: "forbidden", message: "forbidden" } }),
    );
    expect(forbidden).not.toBeInstanceOf(RateLimitedError);
    expect(forbidden).toMatchObject({
      name: "ChakraMCPError",
      status: 403,
      retry_after_secs: undefined,
    });

    const unavailable = await invokeError(
      sdkReturning(
        503,
        { error: { code: "unavailable", message: "try later" } },
        { "retry-after": "5" },
      ),
    );
    expect(unavailable).toMatchObject({ name: "ChakraMCPError", status: 503, retry_after_secs: 5 });
  });
});
