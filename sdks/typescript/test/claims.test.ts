/**
 * Claims and deadlines (relay 0.7+): `release` hands a claim back,
 * `expires_in` shortens a call's deadline, and invocations carry `peer`,
 * `expires_at`, `lease_until` and `deliveries`.
 */
import { describe, expect, it } from "vitest";
import { ChakraMCP, ChakraMCPError, type Invocation, type InvokeResponse } from "../src/index.js";

interface Sent {
  method: string;
  path: string;
  body: unknown;
}

/** A relay that answers every request with `status` and `body`. */
function relay(status: number, body: unknown): { sdk: ChakraMCP; sent: Sent[] } {
  const sent: Sent[] = [];
  const sdk = new ChakraMCP({
    apiKey: "ck_test",
    relayUrl: "http://relay",
    fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
      const req = new Request(input, init);
      const text = await req.text();
      sent.push({
        method: req.method,
        path: new URL(req.url).pathname,
        body: text ? JSON.parse(text) : undefined,
      });
      return new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch,
  });
  return { sdk, sent };
}

describe("claims and deadlines", () => {
  it("release posts to the invocation and returns it", async () => {
    const r = relay(200, { id: "inv-1", status: "pending", deliveries: 0, lease_until: null });
    const inv: Invocation = await r.sdk.inbox.release("inv-1");
    expect(r.sent[0]).toEqual({ method: "POST", path: "/v1/invocations/inv-1/release", body: {} });
    expect(inv.status).toBe("pending");
    expect(inv.deliveries).toBe(0);
  });

  it("a refused release throws with the relay's code", async () => {
    const r = relay(409, {
      error: { code: "chk.invocation.not_releasable", message: "only a claimed invocation can be released" },
    });
    const err = await r.sdk.inbox.release("inv-1").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ChakraMCPError);
    expect((err as ChakraMCPError).status).toBe(409);
    expect((err as ChakraMCPError).code).toBe("chk.invocation.not_releasable");
  });

  it("invoke sends expires_in only when set, and reads peer", async () => {
    const peer = { delivery: "pull", online: false, last_seen_at: "2026-10-29T09:12:03Z", wake: true };
    const r = relay(202, { invocation_id: "inv-1", status: "pending", error: null, peer });
    const res: InvokeResponse = await r.sdk.invoke({
      grant_id: "g-1",
      grantee_agent_id: "a-1",
      input: {},
      expires_in: 600,
    });
    expect(res.peer).toEqual(peer);
    expect(r.sent[0]!.body).toEqual({ grant_id: "g-1", grantee_agent_id: "a-1", input: {}, expires_in: 600 });
    await r.sdk.invoke({ capability_id: "c-1", grantee_agent_id: "a-1", input: {} });
    expect(r.sent[1]!.body).toEqual({ capability_id: "c-1", grantee_agent_id: "a-1", input: {} });
  });
});
