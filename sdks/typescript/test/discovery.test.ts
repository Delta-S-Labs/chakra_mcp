/**
 * Discovery, hermetic: a `fetch` stub serves responses recorded from the
 * relay (`sdks/testdata/discovery`).
 */
import { describe, expect, it } from "vitest";
import { ChakraMCP } from "../src/index.js";
import featured from "../../testdata/discovery/featured.json";
import profile from "../../testdata/discovery/profile.json";
import search from "../../testdata/discovery/search.json";
import standing from "../../testdata/discovery/standing.json";

const fixtures: Record<string, unknown> = { featured, profile, search, standing };
const fixture = (name: string): unknown => fixtures[name];

function sdkServing(body: unknown, seen: string[]): ChakraMCP {
  return new ChakraMCP({
    apiKey: "ck_test",
    relayUrl: "http://relay",
    fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
      const req = new Request(input, init);
      seen.push(`${req.method} ${req.url}`);
      return new Response(JSON.stringify(body), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch,
  });
}

describe("discovery", () => {
  it("search sends its filters and decodes a page", async () => {
    const seen: string[] = [];
    const sdk = sdkServing(fixture("search"), seen);
    const page = await sdk.discovery.search({
      q: "guide",
      tags: ["help", "docs"],
      callable: true,
      capability_schema: { properties: { answer: {} } },
      limit: 2,
    });
    const url = new URL(seen[0]!.split(" ")[1]!);
    expect(url.pathname).toBe("/v1/discovery/agents");
    expect(url.searchParams.get("q")).toBe("guide");
    expect(url.searchParams.get("tags")).toBe("help,docs");
    expect(url.searchParams.get("callable")).toBe("true");
    expect(JSON.parse(url.searchParams.get("capability_schema")!)).toEqual({ properties: { answer: {} } });
    expect(url.searchParams.get("limit")).toBe("2");
    const guide = page.agents[0]!;
    expect(guide.slug).toBe("guide");
    expect(guide.account_slug).toBe("chakramcp");
    expect(guide.capabilities[0]).toMatchObject({ name: "guide.ask", access: "public" });
    expect(typeof guide.last_seen_at).toBe("string");
    expect(page.total_estimate).toBe(1);
  });

  it("featured returns the agents", async () => {
    const seen: string[] = [];
    const agents = await sdkServing(fixture("featured"), seen).discovery.featured();
    expect(seen[0]).toBe("GET http://relay/v1/discovery/featured");
    expect(agents.length).toBeGreaterThan(0);
    expect(agents[0]!.featured).toBe(true);
  });

  it("profile takes an id or account/agent", async () => {
    const seen: string[] = [];
    const sdk = sdkServing(fixture("profile"), seen);
    const profile = await sdk.discovery.profile("chakramcp/guide");
    await sdk.discovery.profile("01a1607c-5ca5-727e-96b8-4fcc3b4a18f7");
    expect(seen).toEqual([
      "GET http://relay/v1/discovery/agents/chakramcp/guide",
      "GET http://relay/v1/discovery/agents/01a1607c-5ca5-727e-96b8-4fcc3b4a18f7",
    ]);
    expect(profile.owner.account_slug).toBe("chakramcp");
    expect(profile.capabilities[0]!.access).toBe("public");
    expect(profile.capabilities[0]!.input_schema).toBeTypeOf("object");
    expect(profile.reviews.latest[0]!.by).toBe("An agent");
    expect(profile.how_to_call.cli).toContain("chakramcp invoke ensure chakramcp/guide");
  });

  it("standing names the agent in the query", async () => {
    const seen: string[] = [];
    const you = await sdkServing(fixture("standing"), seen).discovery.standing("bob/venue-bot");
    expect(seen[0]).toBe("GET http://relay/v1/discovery/standing?agent=bob%2Fvenue-bot");
    expect(you.callable.map((c) => c.via)).toEqual(["grant", "public"]);
    expect(you.callable[1]!.as_agent_id).toBeNull();
    expect(you.can_start_call_as).toHaveLength(1);
  });

  it("agents.create sends tags", async () => {
    let sent: unknown;
    const sdk = new ChakraMCP({
      apiKey: "ck_test",
      relayUrl: "http://relay",
      fetch: (async (input: RequestInfo | URL, init?: RequestInit) => {
        sent = JSON.parse(String(init?.body));
        return new Response(JSON.stringify({ id: "a1", tags: ["booking"] }), { status: 200 });
      }) as typeof fetch,
    });
    const agent = await sdk.agents.create({
      account_id: "acc1",
      slug: "rooms",
      display_name: "Rooms",
      tags: ["booking"],
    });
    expect(sent).toMatchObject({ tags: ["booking"] });
    expect(agent.tags).toEqual(["booking"]);
  });
});
