import { expect, test } from "@playwright/test";
import { deriveGetStarted, guideCommand, publishCommand } from "./get-started";
import type { Agent, FeaturedAgent, Invocation } from "./relay";

function agent(over: Partial<Agent> = {}): Agent {
  return {
    id: "a1",
    account_id: "acct",
    account_slug: "me",
    account_display_name: "Me",
    slug: "hermes",
    display_name: "Hermes",
    description: "",
    visibility: "network",
    endpoint_url: null,
    created_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-01T00:00:00Z",
    is_mine: true,
    capability_count: 0,
    avg_rating: null,
    review_count: 0,
    last_polled_at: null,
    network_capability_count: 0,
    ...over,
  };
}

const invocation = (over: Partial<Invocation>) => ({ i_invoked: false, i_served: false, ...over }) as Invocation;

const guide: FeaturedAgent = {
  id: "guide-id",
  account_slug: "chakramcp",
  slug: "guide",
  display_name: "ChakraMCP Guide",
  description: "",
  online: true,
  featured: true,
  public_capabilities: [{ id: "cap-guide", name: "guide.ask", description: "" }],
};

const done = (g: ReturnType<typeof deriveGetStarted>) => g.steps.filter((s) => s.done).map((s) => s.id);

test("a new account has everything to do", () => {
  const g = deriveGetStarted({ agents: [], invocations: [], featured: [] });
  expect(done(g)).toEqual([]);
  expect(g.allDone).toBe(false);
  expect(g.guide).toBeNull();
  expect(g.fromAgentId).toBeNull();
});

test("an agent that has polled is connected", () => {
  const g = deriveGetStarted({
    agents: [agent({ last_polled_at: "2026-10-02T00:00:00Z" })],
    invocations: [],
    featured: [guide],
  });
  expect(done(g)).toEqual(["create", "connect"]);
  expect(g.guide).toEqual({ agentId: "guide-id", agentName: "ChakraMCP Guide", capabilityId: "cap-guide" });
});

test("making a call counts as connected; serving one doesn't count as calling", () => {
  expect(done(deriveGetStarted({ agents: [agent()], invocations: [invocation({ i_served: true })], featured: [] }))).toEqual([
    "create",
  ]);
  expect(done(deriveGetStarted({ agents: [agent()], invocations: [invocation({ i_invoked: true })], featured: [] }))).toEqual([
    "create",
    "connect",
    "call",
  ]);
});

test("all four done", () => {
  const g = deriveGetStarted({
    agents: [agent({ network_capability_count: 1 })],
    invocations: [invocation({ i_invoked: true })],
    featured: [],
  });
  expect(g.allDone).toBe(true);
  expect(g.doneCount).toBe(4);
});

test("calls come from the oldest agent, and only a featured guide.ask counts", () => {
  const g = deriveGetStarted({
    agents: [agent({ id: "new", created_at: "2026-10-05T00:00:00Z" }), agent({ id: "old" })],
    invocations: [],
    featured: [{ ...guide, featured: false }],
  });
  expect(g.fromAgentId).toBe("old");
  expect(g.guide).toBeNull();
});

test("the CLI command", () => {
  expect(guideCommand("cap-guide", "a1")).toBe(
    `chakramcp invoke \\\n  --capability cap-guide \\\n  --as a1 \\\n  --input '{"question": "How do grants work?"}' \\\n  --wait`,
  );
  expect(guideCommand("cap-guide", null)).toContain("--as <your-agent-id>");
});

test("the publish command", () => {
  expect(publishCommand("a1")).toBe("chakramcp capabilities add \\\n  --agent a1 \\\n  --template message_owner");
});
