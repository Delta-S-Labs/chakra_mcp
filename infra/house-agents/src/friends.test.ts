import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChakraMCP, Friendship, Grant } from "@chakramcp/sdk";
import type { HouseAgent } from "./agents.js";
import { syncFriends } from "./friends.js";

const HOUSE: HouseAgent[] = [
  { id: "ping-id", slug: "ping", capabilityId: "cap-ping", capabilityName: "ping.echo" },
  { id: "guide-id", slug: "guide", capabilityId: "cap-guide", capabilityName: "guide.ask" },
];

const agent = (id: string, account = `acct-${id}`) => ({
  id,
  slug: id,
  display_name: id,
  account_id: account,
  account_slug: account,
  account_display_name: account,
});

function friendship(id: string, proposer: string, target: string, status: Friendship["status"]): Friendship {
  return {
    id,
    status,
    proposer: agent(proposer),
    target: agent(target),
    proposer_message: null,
    response_message: null,
    counter_of_id: null,
    created_at: "",
    updated_at: "",
    decided_at: null,
    i_proposed: false,
    i_received: true,
  } as Friendship;
}

function fakeRelay(friendships: Friendship[], grants: Array<Pick<Grant, "granter" | "grantee" | "capability_id">>) {
  const accepted: string[] = [];
  const created: string[] = [];
  const chakra = {
    friendships: {
      list: async () => friendships,
      accept: async (id: string) => {
        accepted.push(id);
        const f = friendships.find((x) => x.id === id)!;
        f.status = "accepted";
        return f;
      },
    },
    grants: { list: async () => grants },
    relayRequest: async (
      method: string,
      path: string,
      body: { granter_agent_id: string; grantee_agent_id: string; capability_id: string },
    ) => {
      assert.equal(`${method} ${path}`, "POST /v1/grants");
      created.push(`${body.granter_agent_id}->${body.grantee_agent_id}:${body.capability_id}`);
      return {};
    },
  } as unknown as ChakraMCP;
  return { chakra, accepted, created };
}

test("accepts proposals to house agents and grants each friend the capability", async () => {
  const { chakra, accepted, created } = fakeRelay(
    [
      friendship("f1", "alice", "guide-id", "proposed"),
      friendship("f2", "bob", "ping-id", "accepted"),
      friendship("f3", "carol", "guide-id", "accepted"),
      friendship("f4", "dave", "someone-else", "proposed"),
      friendship("f5", "mallory", "guide-id", "proposed"),
      friendship("f6", "ping-id", "guide-id", "accepted"),
    ],
    // Carol already has her grant.
    [{ granter: agent("guide-id"), grantee: agent("carol"), capability_id: "cap-guide" }],
  );
  await syncFriends(chakra, HOUSE, { agents: new Set(["mallory"]), accounts: new Set() });
  assert.deepEqual(accepted, ["f1"]);
  assert.deepEqual(created.sort(), ["guide-id->alice:cap-guide", "ping-id->bob:cap-ping"]);
});

test("a blocked account gets nothing", async () => {
  const { chakra, accepted, created } = fakeRelay([friendship("f1", "eve", "guide-id", "proposed")], []);
  await syncFriends(chakra, HOUSE, { agents: new Set(), accounts: new Set(["acct-eve"]) });
  assert.deepEqual(accepted, []);
  assert.deepEqual(created, []);
});
