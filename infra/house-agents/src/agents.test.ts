import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChakraMCP } from "@chakramcp/sdk";
import { canonical, ensureAgents, GUIDE, PING } from "./agents.js";

test("canonical JSON ignores key order", () => {
  assert.equal(canonical({ b: 1, a: { d: [1, { f: 1, e: 2 }], c: 2 } }), canonical({ a: { c: 2, d: [1, { e: 2, f: 1 }] }, b: 1 }));
  assert.notEqual(canonical({ a: 1 }), canonical({ a: 2 }));
});

function fakeRelay(existing: { agents: Array<Record<string, unknown>>; caps: Record<string, Array<Record<string, unknown>>> }) {
  const calls: string[] = [];
  let n = 0;
  const chakra = {
    me: async () => ({ memberships: [{ account_id: "house-acct", slug: "chakramcp" }] }),
    agents: {
      list: async () => existing.agents,
      create: async (body: Record<string, unknown>) => {
        calls.push(`create ${String(body.slug)}`);
        return { ...body, id: `agent-${++n}` };
      },
      update: async (id: string, body: Record<string, unknown>) => {
        calls.push(`update ${id}`);
        return { ...existing.agents.find((a) => a.id === id), ...body };
      },
      capabilities: {
        list: async (agentId: string) => existing.caps[agentId] ?? [],
        create: async (_agentId: string, body: Record<string, unknown>) => {
          calls.push(`create ${String(body.name)}`);
          return { ...body, id: `cap-${++n}` };
        },
      },
    },
    relayRequest: async (method: string, path: string) => {
      calls.push(`${method} ${path}`);
      return { id: path.split("/").pop() };
    },
  } as unknown as ChakraMCP;
  return { chakra, calls };
}

test("creates both agents and capabilities when missing", async () => {
  const { chakra, calls } = fakeRelay({ agents: [], caps: {} });
  const [ping, guide] = await ensureAgents(chakra, "chakramcp");
  assert.deepEqual(calls, ["create ping", "create ping.echo", "create guide", "create guide.ask"]);
  assert.equal(ping.capabilityName, "ping.echo");
  assert.equal(guide.capabilityName, "guide.ask");
});

test("leaves matching agents alone and corrects drifted ones", async () => {
  const cap = (spec: typeof PING, id: string) => ({
    id,
    name: spec.capability.name,
    description: spec.capability.description,
    // Key order as jsonb might return it.
    input_schema: JSON.parse(canonical(spec.capability.input_schema)),
    output_schema: JSON.parse(canonical(spec.capability.output_schema)),
    visibility: "network",
    public_invoke: true,
    public_monthly_quota_per_agent: spec.capability.public_monthly_quota_per_agent,
  });
  const { chakra, calls } = fakeRelay({
    agents: [
      { id: "p", account_id: "house-acct", slug: "ping", display_name: PING.display_name, description: PING.description, visibility: "network" },
      { id: "g", account_id: "house-acct", slug: "guide", display_name: "Old name", description: GUIDE.description, visibility: "network" },
    ],
    caps: { p: [cap(PING, "cp")], g: [{ ...cap(GUIDE, "cg"), public_monthly_quota_per_agent: 5 }] },
  });
  await ensureAgents(chakra, "chakramcp");
  assert.deepEqual(calls, ["update g", "PATCH /v1/agents/g/capabilities/cg"]);
});

test("refuses an account the key can't act for", async () => {
  const { chakra } = fakeRelay({ agents: [], caps: {} });
  await assert.rejects(ensureAgents(chakra, "someone-else"), /isn't a member/);
});
