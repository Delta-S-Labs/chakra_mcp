import Anthropic from "@anthropic-ai/sdk";
import type {
  BetaMessage,
  BetaMessageParam,
} from "@anthropic-ai/sdk/resources/beta/messages/messages";
import { log, errorName } from "./log.js";
import { costUsd, worstCaseUsd } from "./pricing.js";
import type { Sessions } from "./sessions.js";

export const GUIDE_MAX_TOKENS = 3000;
/** Server-side compaction summarizes a conversation once a turn's input passes this. */
export const COMPACT_AT_INPUT_TOKENS = 60_000;
export const CALL_TIMEOUT_MS = 120_000;

/** One Claude request over a whole conversation. */
export interface Claude {
  call(messages: BetaMessageParam[]): Promise<BetaMessage>;
  /** Free: checks the request shape and counts the system prompt's tokens. */
  check(): Promise<number>;
}

export function makeClaude(client: Anthropic, model: string, system: string): Claude {
  const shared = {
    model,
    betas: ["compact-2026-01-12"],
    // Docs questions don't need deep reasoning.
    output_config: { effort: "low" as const },
    // Caches the docs once for everyone, and each conversation so far.
    system: [{ type: "text" as const, text: system, cache_control: { type: "ephemeral" as const } }],
    cache_control: { type: "ephemeral" as const },
    context_management: {
      edits: [
        {
          type: "compact_20260112" as const,
          trigger: { type: "input_tokens" as const, value: COMPACT_AT_INPUT_TOKENS },
        },
      ],
    },
  };
  return {
    call(messages) {
      return client.beta.messages
        .stream(
          { ...shared, max_tokens: GUIDE_MAX_TOKENS, messages },
          { signal: AbortSignal.timeout(CALL_TIMEOUT_MS) },
        )
        .finalMessage();
    },
    async check() {
      const res = await client.beta.messages.countTokens(
        { ...shared, messages: [{ role: "user", content: "ping" }] },
        { signal: AbortSignal.timeout(30_000) },
      );
      return res.input_tokens;
    },
  };
}

/** A question the Guide couldn't answer, with what to tell the asker. */
export class GuideError extends Error {
  constructor(
    readonly userMessage: string,
    readonly retryAfterSecs?: number,
  ) {
    super(userMessage);
    this.name = "GuideError";
  }
}

export interface Answer {
  text: string;
  /** Claude hit the output limit before finishing. */
  cutShort: boolean;
  turn: number;
  newSession: boolean;
}

/** What an attempt cost when it failed. Rejected requests cost nothing. */
function failedAttemptUsd(err: unknown, messages: BetaMessageParam[]): number {
  if (err instanceof Anthropic.APIError && err.status !== undefined && err.status < 500) return 0;
  return worstCaseUsd(JSON.stringify(messages).length, GUIDE_MAX_TOKENS);
}

function toGuideError(err: unknown): GuideError {
  if (err instanceof Anthropic.RateLimitError || (err instanceof Anthropic.APIError && err.status === 529)) {
    return new GuideError("The Guide is busy. Try again in a minute.", 60);
  }
  if (err instanceof Anthropic.AuthenticationError || err instanceof Anthropic.PermissionDeniedError) {
    return new GuideError("The Guide is unavailable right now. Try again later.", 3600);
  }
  if (err instanceof Anthropic.BadRequestError) {
    return new GuideError("The Guide couldn't answer that. Try again with `new_session: true`.");
  }
  if (err instanceof Anthropic.APIUserAbortError || err instanceof Anthropic.APIConnectionTimeoutError) {
    return new GuideError("The Guide took too long to answer. Try again.", 30);
  }
  return new GuideError("The Guide hit an error. Try again in a minute.", 60);
}

/**
 * Asks Claude one question in the calling agent's conversation and saves the
 * turn. `onCost` hears every attempt's cost, failed ones included.
 */
export async function ask(
  claude: Claude,
  sessions: Sessions,
  agentId: string,
  question: string,
  startNew: boolean,
  onCost: (usd: number) => void,
): Promise<Answer> {
  let session = sessions.load(agentId, startNew);
  for (let attempt = 0; ; attempt++) {
    const messages: BetaMessageParam[] = [
      ...session.messages,
      { role: "user", content: [{ type: "text", text: question }] },
    ];
    let reply: BetaMessage;
    try {
      reply = await claude.call(messages);
    } catch (err) {
      onCost(failedAttemptUsd(err, messages));
      if (attempt === 0 && !session.isNew && err instanceof Anthropic.BadRequestError) {
        // The stored history no longer replays (an API or model change).
        // Start this agent's conversation over, once.
        log.warn("guide.session_reset", { agent_id: agentId, status: err.status });
        session = { messages: [], turns: 0, isNew: true };
        continue;
      }
      log.warn("guide.claude_failed", {
        agent_id: agentId,
        error: errorName(err),
        status: err instanceof Anthropic.APIError ? (err.status ?? null) : null,
      });
      throw toGuideError(err);
    }
    onCost(costUsd(reply.usage));

    if (reply.stop_reason === "refusal") {
      // Not saved: the conversation goes on as if this question wasn't asked.
      throw new GuideError("The Guide can't help with that question.");
    }
    const text = reply.content
      .flatMap((b) => (b.type === "text" ? [b.text] : []))
      .join("\n\n")
      .trim();
    // The whole reply goes back, compaction and thinking blocks included.
    messages.push({ role: "assistant", content: reply.content });
    const turn = session.turns + 1;
    sessions.save(agentId, messages, turn, session.isNew);
    return {
      text: text || "The Guide had nothing to add. Try rephrasing the question.",
      cutShort: reply.stop_reason === "max_tokens",
      turn,
      newSession: session.isNew,
    };
  }
}
