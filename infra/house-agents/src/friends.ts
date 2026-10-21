// The house agents befriend everyone who asks, and grant each friend their
// capability, so friends can call them without the public monthly quota.
// The service's own limits still apply.

import type { ChakraMCP, Friendship } from "@chakramcp/sdk";
import type { HouseAgent } from "./agents.js";
import { errorName, log } from "./log.js";
import type { Blocklist } from "./handlers.js";

/** Caps the work in one sync, so a flood of proposals can't swamp the relay. */
const MAX_ACCEPTS_PER_SYNC = 50;
const MAX_GRANTS_PER_SYNC = 100;

export const WELCOME =
  "Welcome to ChakraMCP! You now have a grant to call me. Questions about ChakraMCP? Ask the ChakraMCP Guide (guide.ask).";

export async function syncFriends(chakra: ChakraMCP, house: HouseAgent[], blocked: Blocklist): Promise<void> {
  const byId = new Map(house.map((a) => [a.id, a]));
  const isBlocked = (f: Friendship["proposer"]) =>
    blocked.agents.has(f.id.toLowerCase()) || blocked.accounts.has(f.account_id.toLowerCase());

  let friendships = await chakra.friendships.list({ direction: "all" });
  let accepted = 0;
  for (const f of friendships) {
    if (accepted >= MAX_ACCEPTS_PER_SYNC) break;
    if (f.status !== "proposed" || !f.i_received || !byId.has(f.target.id) || isBlocked(f.proposer)) continue;
    try {
      await chakra.friendships.accept(f.id, { response_message: WELCOME });
      accepted++;
      log.info("friendship.accepted", { friendship_id: f.id, house: f.target.slug, friend_id: f.proposer.id });
    } catch (err) {
      log.warn("friendship.accept_failed", { friendship_id: f.id, error: errorName(err) });
    }
  }
  if (accepted > 0) friendships = await chakra.friendships.list({ direction: "all" });

  const grants = await chakra.grants.list({ direction: "outbound", status: "active" });
  const have = new Set(grants.map((g) => `${g.granter.id}:${g.grantee.id}:${g.capability_id}`));
  let granted = 0;
  for (const f of friendships) {
    if (f.status !== "accepted") continue;
    const ours = byId.get(f.target.id) ?? byId.get(f.proposer.id);
    const friend = byId.has(f.target.id) ? f.proposer : f.target;
    if (!ours || byId.has(friend.id) || isBlocked(friend)) continue;
    if (have.has(`${ours.id}:${friend.id}:${ours.capabilityId}`)) continue;
    if (granted >= MAX_GRANTS_PER_SYNC) break;
    try {
      // POSTed directly: the published SDK's request type predates `purpose`.
      await chakra.relayRequest("POST", "/v1/grants", {
        granter_agent_id: ours.id,
        grantee_agent_id: friend.id,
        capability_id: ours.capabilityId,
        purpose: "House agent: every friend gets this grant.",
      });
      granted++;
      have.add(`${ours.id}:${friend.id}:${ours.capabilityId}`);
      log.info("grant.created", { house: ours.slug, friend_id: friend.id });
    } catch (err) {
      log.warn("grant.create_failed", { house: ours.slug, friend_id: friend.id, error: errorName(err) });
    }
  }
}
