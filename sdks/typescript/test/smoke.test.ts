/**
 * Hermetic smoke - a `fetch` stub stands in for the real backend so the
 * test suite runs in CI without a server. Validates that the SDK shapes
 * requests + headers correctly and surfaces error envelopes.
 */
import { describe, expect, it, vi } from "vitest";
import { ChakraMCP, ChakraMCPError } from "../src/index.js";

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
