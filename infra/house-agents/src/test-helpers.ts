// Fakes shared by the tests.
import type { ChakraMCP } from "@chakramcp/sdk";
import type { BetaMessage, BetaMessageParam } from "@anthropic-ai/sdk/resources/beta/messages/messages";
import type { Claude } from "./guide.js";
import type { HouseInvocation, Outcome } from "./relay.js";

export function clock(start = 1_800_000_000_000): { now: () => number; advance: (ms: number) => void } {
  let t = start;
  return { now: () => t, advance: (ms) => void (t += ms) };
}

export function reply(text: string, extra: Partial<BetaMessage> = {}): BetaMessage {
  return {
    id: "msg_test",
    type: "message",
    role: "assistant",
    model: "claude-sonnet-5-5",
    content: [{ type: "text", text, citations: null }],
    stop_reason: "end_turn",
    stop_sequence: null,
    usage: {
      input_tokens: 1000,
      output_tokens: 100,
      cache_creation_input_tokens: 0,
      cache_read_input_tokens: 0,
    },
    ...extra,
  } as unknown as BetaMessage;
}

/** A Claude that answers from a script and records what it was sent. */
export function fakeClaude(script: Array<BetaMessage | Error>): Claude & { calls: BetaMessageParam[][] } {
  const calls: BetaMessageParam[][] = [];
  return {
    calls,
    check: async () => 30_000,
    async call(messages) {
      calls.push(structuredClone(messages));
      const next = script.shift();
      if (!next) throw new Error("fakeClaude: script ran out");
      if (next instanceof Error) throw next;
      return next;
    },
  };
}

export function invocation(over: Partial<HouseInvocation> = {}): HouseInvocation {
  return {
    id: `inv-${Math.random().toString(36).slice(2)}`,
    grant_id: null,
    granter_agent_id: "guide-id",
    granter_display_name: "ChakraMCP Guide",
    grantee_agent_id: "caller-1",
    grantee_display_name: "Caller",
    grantee_account_id: "acct-1",
    capability_id: "cap-guide",
    capability_name: "guide.ask",
    status: "in_progress",
    elapsed_ms: 0,
    error_message: null,
    input_preview: { question: "How do I register an agent?" },
    output_preview: null,
    created_at: new Date(0).toISOString(),
    claimed_at: null,
    i_served: true,
    i_invoked: false,
    tier: "public",
    ...over,
  } as HouseInvocation;
}

/** A relay client that records posted results. */
export function fakeChakra(): { chakra: ChakraMCP; results: Map<string, Outcome> } {
  const results = new Map<string, Outcome>();
  const chakra = {
    inbox: {
      respond: async (id: string, outcome: Outcome) => {
        results.set(id, outcome);
        return {};
      },
    },
  } as unknown as ChakraMCP;
  return { chakra, results };
}
