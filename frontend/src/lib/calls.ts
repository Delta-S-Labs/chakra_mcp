// Calls in words, for the owner's view: which side your agents are on, how
// a call ended, what an event entry says, and what happens to the content.

import type { Call, CallEndedBy, CallEntry, CallParty, CallState } from "./relay";

export type Side = "caller" | "callee";

/** The side your agents are on: `both` when both agents are yours. */
export function mySide(call: Call, myAgentIds: ReadonlySet<string>): Side | "both" | null {
  const caller = !!call.caller.agent_id && myAgentIds.has(call.caller.agent_id);
  const callee = !!call.callee.agent_id && myAgentIds.has(call.callee.agent_id);
  if (caller && callee) return "both";
  if (caller) return "caller";
  if (callee) return "callee";
  return null;
}

/** The party across from `side` (the callee when both are yours). */
export function otherParty(call: Call, side: Side | "both" | null): CallParty {
  return side === "callee" ? call.caller : call.callee;
}

/** A party's name, falling back for an agent that's been deleted. */
export function partyName(p: CallParty): string {
  if (p.display_name) return p.display_name;
  return p.agent_id ? "An agent" : "A deleted agent";
}

const STATES: Record<CallState, string> = {
  ringing: "Ringing",
  active: "Active",
  declined: "Declined",
  missed: "Missed",
  ended: "Ended",
};

export function stateLabel(state: CallState): string {
  return STATES[state] ?? state;
}

const ENDED_BY: Record<CallEndedBy, string> = {
  caller: "the caller hung up",
  callee: "the callee hung up",
  owner: "an owner ended it",
  idle: "no messages for a while",
  gone: "one of the agents was deleted",
};

/** Why an ended call ended, e.g. "Ended: an owner ended it". */
export function endedLabel(call: Call): string | null {
  if (call.state !== "ended" || !call.ended_by) return null;
  return `Ended: ${ENDED_BY[call.ended_by] ?? call.ended_by}`;
}

export function memoryLabel(call: Call): string {
  if (call.memory === "own") return "Each agent keeps its own memory";
  return call.memory_features.tags
    ? "The relay keeps the call's memory"
    : "The relay keeps the latest messages";
}

function day(iso: string): string {
  return new Date(iso).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
}

/** What happens to the call's content (spec §9). */
export function retentionLabel(call: Call): string {
  if (call.purged_at) return `Content deleted on ${day(call.purged_at)}`;
  if (call.purge_at) {
    return new Date(call.purge_at).getTime() <= Date.now()
      ? "Content being deleted"
      : `Content kept until ${day(call.purge_at)}`;
  }
  if (call.ttl_secs === 0) return "Content deleted when the call ends";
  const days = Math.round(call.ttl_secs / 86_400);
  if (days >= 1) return `Content kept ${days} day${days === 1 ? "" : "s"} after the call ends`;
  const hours = Math.max(1, Math.round(call.ttl_secs / 3_600));
  return `Content kept ${hours} hour${hours === 1 ? "" : "s"} after the call ends`;
}

/** The party who sent an entry, by agent id. */
export function senderOf(call: Call, entry: CallEntry): CallParty | null {
  if (!entry.from_agent_id) return null;
  if (entry.from_agent_id === call.caller.agent_id) return call.caller;
  if (entry.from_agent_id === call.callee.agent_id) return call.callee;
  return null;
}

/** One line for an event entry (anything but a message). */
export function eventLine(call: Call, entry: CallEntry): string {
  const who = senderOf(call, entry);
  const name = who ? partyName(who) : "The relay";
  switch (entry.kind) {
    case "invite":
      return `${name} called ${partyName(call.callee)}`;
    case "accepted":
      return `${name} answered`;
    case "declined":
      return entry.text ? `${name} declined: “${entry.text}”` : `${name} declined`;
    case "missed":
      return "Nobody answered";
    case "ended":
      if (!who) {
        return call.ended_by === "owner"
          ? "An owner ended the call"
          : call.ended_by === "gone"
            ? "The call ended: an agent was deleted"
            : "The call ended after a quiet spell";
      }
      return `${name} hung up`;
    case "invocation": {
      const what = entry.invocation?.capability ?? "a capability";
      const status = entry.invocation?.status;
      return `${name} asked for ${what}${status ? ` (${status.replace(/_/g, " ")})` : ""}`;
    }
    default:
      return entry.kind;
  }
}

/** Whether an owner may still end the call, or delete its content. */
export function canEnd(call: Call): boolean {
  return call.state === "ringing" || call.state === "active";
}

export function canDelete(call: Call): boolean {
  return !call.purged_at;
}
