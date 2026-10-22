// What each house agent does with an invocation: checks, limits, then the
// answer. Every path ends in a posted result; nothing throws out of here.

import type { ChakraMCP } from "@chakramcp/sdk";
import { GUIDE_INPUT_MAX_BYTES, PING_MESSAGE_MAX, QUESTION_MAX_CHARS, type HouseAgent } from "./agents.js";
import { ask, GuideError, type Claude } from "./guide.js";
import type { KeyedLock, Semaphore } from "./keyed-queue.js";
import { PING_LIMITS, type Limiter, type Limits } from "./limits.js";
import { errorName, log } from "./log.js";
import { failed, respond, type HouseInvocation, type Outcome } from "./relay.js";
import type { Sessions } from "./sessions.js";

export const OUTPUT_MAX_BYTES = 12 * 1024;
/** Questions one agent may have running or waiting. */
export const MAX_PENDING_PER_AGENT = 2;
/** Questions all agents together may have running or waiting. */
export const MAX_PENDING_TOTAL = 8;
const RESPOND_DEADLINE_MS = 60_000;
const RESPOND_DEADLINE_STOPPING_MS = 20_000;

export interface Blocklist {
  agents: ReadonlySet<string>;
  accounts: ReadonlySet<string>;
}

interface Caller {
  agentId: string;
  accountId: string | null;
}

/** Why a call was turned away, for the logs (never the call's content). */
export type RefusalReason = "capability" | "no_caller" | "blocked" | "input" | "busy" | "limit" | "restarting";

/** A refusal: what to tell the caller, and what to log. */
interface Refusal {
  reason: RefusalReason;
  message: string;
  limit?: string;
}

function refusal(reason: RefusalReason, message: string, limit?: string): Refusal {
  return { reason, message, ...(limit ? { limit } : {}) };
}

/** Who's calling, or why the call is refused. */
function caller(inv: HouseInvocation, house: HouseAgent, blocked: Blocklist): Caller | Refusal {
  if (inv.capability_name !== house.capabilityName) {
    return refusal("capability", `${house.slug} has no capability "${inv.capability_name}".`);
  }
  const agentId = inv.grantee_agent_id;
  if (!agentId) return refusal("no_caller", "The relay didn't say which agent is calling.");
  const accountId = inv.grantee_account_id ?? null;
  if (blocked.agents.has(agentId.toLowerCase()) || (accountId && blocked.accounts.has(accountId.toLowerCase()))) {
    return refusal("blocked", "This agent can't call the house agents.");
  }
  return { agentId, accountId };
}

function isRefusal(v: Caller | Refusal): v is Refusal {
  return "reason" in v;
}

/** Logs a refusal: ids and the reason category only. */
function logRefusal(event: string, inv: HouseInvocation, r: Refusal): void {
  log.info(event, {
    invocation_id: inv.id,
    agent_id: inv.grantee_agent_id ?? null,
    reason: r.reason,
    limit: r.limit ?? null,
  });
}

function limited(reason: string, retryAfterSecs: number): string {
  return `Limit reached: ${reason}. Try again in ${retryAfterSecs} s.`;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * An A2A call's input is its SendMessage params: `{message: {parts: [...]}}`.
 * Returns the message's data part if it has one, else its text, else null.
 */
export function fromA2a(input: Record<string, unknown>): { data?: Record<string, unknown>; text?: string } | null {
  const msg = input.message;
  if (typeof msg === "string") return { text: msg };
  if (!isRecord(msg) || !Array.isArray(msg.parts)) return null;
  const texts: string[] = [];
  for (const part of msg.parts as unknown[]) {
    if (!isRecord(part)) continue;
    const kind = part.kind ?? part.type;
    if (kind === "data" && isRecord(part.data)) return { data: part.data };
    if (kind === "text" && typeof part.text === "string") texts.push(part.text);
  }
  return texts.length > 0 ? { text: texts.join("\n") } : null;
}

// ─── Ping ───────────────────────────────────────────────

export function pingOutput(input: unknown, now: Date): Record<string, unknown> {
  const out: Record<string, unknown> = { pong: true, answered_at: now.toISOString() };
  const a2a = isRecord(input) ? fromA2a(input) : null;
  const message = a2a?.text ?? (typeof a2a?.data?.message === "string" ? a2a.data.message : undefined);
  if (message !== undefined) out.message = message.slice(0, PING_MESSAGE_MAX);
  return out;
}

export interface PingDeps {
  chakra: ChakraMCP;
  house: HouseAgent;
  limiter: Limiter;
  blocked: Blocklist;
}

export async function handlePing(deps: PingDeps, inv: HouseInvocation): Promise<void> {
  const who = caller(inv, deps.house, deps.blocked);
  let outcome: Outcome;
  let refused: Refusal | null = null;
  if (isRefusal(who)) {
    refused = who;
  } else {
    const verdict = deps.limiter.check(deps.house.capabilityName, who.agentId, who.accountId, PING_LIMITS);
    if (!verdict.ok) {
      refused = refusal("limit", limited(verdict.reason, verdict.retryAfterSecs), verdict.limit);
    } else {
      deps.limiter.record(deps.house.capabilityName, who.agentId, who.accountId);
    }
  }
  if (refused) {
    logRefusal("ping.refused", inv, refused);
    outcome = failed(refused.message);
  } else {
    log.info("ping.answered", { invocation_id: inv.id, agent_id: inv.grantee_agent_id ?? null });
    outcome = { status: "succeeded", output: pingOutput(inv.input_preview, new Date()) };
  }
  await respond(deps.chakra, inv, outcome, RESPOND_DEADLINE_MS);
}

// ─── Guide ──────────────────────────────────────────────

export type GuideInput = { question: string; newSession: boolean };

export function parseGuideInput(raw: unknown): GuideInput | string {
  let input: unknown = typeof raw === "string" ? { question: raw } : raw;
  if (!isRecord(input)) return 'Send an object like {"question": "How do I register an agent?"}.';
  if (input.__chakramcp_truncated__ === true || Buffer.byteLength(JSON.stringify(input)) > GUIDE_INPUT_MAX_BYTES) {
    return `The input is too large: ask in at most ${QUESTION_MAX_CHARS} characters.`;
  }
  if (!("question" in input) && "message" in input) {
    const a2a = fromA2a(input);
    if (a2a) input = a2a.data ?? { question: a2a.text };
  }
  const fields = input as Record<string, unknown>;
  const question = typeof fields.question === "string" ? fields.question.trim() : "";
  if (!question) return "`question` is required: a non-empty string.";
  if (question.length > QUESTION_MAX_CHARS) return `\`question\` is too long: at most ${QUESTION_MAX_CHARS} characters.`;
  if (fields.new_session !== undefined && typeof fields.new_session !== "boolean") {
    return "`new_session` must be true or false.";
  }
  return { question, newSession: fields.new_session === true };
}

/** The Guide's output, cut to fit what the relay keeps whole. */
export function guideOutput(text: string, cutShort: boolean, turn: number, newSession: boolean): Record<string, unknown> {
  const build = (answer: string, cut: boolean) => ({
    answer,
    cut_short: cut,
    conversation: { turn, new_session: newSession },
  });
  let out = build(text, cutShort);
  let bytes = Buffer.byteLength(JSON.stringify(out));
  let answer = text;
  while (bytes > OUTPUT_MAX_BYTES) {
    answer = answer.slice(0, Math.floor((answer.length * OUTPUT_MAX_BYTES) / bytes) - 200);
    out = build(`${answer}\n\n[Cut to fit the 12 KB limit. Ask a narrower question.]`, true);
    bytes = Buffer.byteLength(JSON.stringify(out));
  }
  return out;
}

export interface GuideDeps {
  chakra: ChakraMCP;
  house: HouseAgent;
  limiter: Limiter;
  limits: Limits;
  sessions: Sessions;
  claude: Claude;
  blocked: Blocklist;
  lock: KeyedLock;
  slots: Semaphore;
  /** Set once shutdown begins: queued questions are turned away. */
  stopping: () => boolean;
}

/**
 * Takes a Guide question. Refusals are posted straight away; an accepted
 * question is queued behind the caller's previous one and the returned
 * promise settles once its answer is posted.
 */
export async function handleGuide(deps: GuideDeps, inv: HouseInvocation): Promise<void> {
  const deadline = () => (deps.stopping() ? RESPOND_DEADLINE_STOPPING_MS : RESPOND_DEADLINE_MS);
  const refuse = (r: Refusal) => {
    logRefusal("guide.refused", inv, r);
    return respond(deps.chakra, inv, failed(r.message), deadline()).then(() => undefined);
  };

  const who = caller(inv, deps.house, deps.blocked);
  if (isRefusal(who)) return refuse(who);
  const input = parseGuideInput(inv.input_preview);
  if (typeof input === "string") return refuse(refusal("input", input));
  if (deps.lock.pending(who.agentId) >= MAX_PENDING_PER_AGENT) {
    return refuse(refusal("busy", "One question at a time: wait for the Guide's answer to your previous question."));
  }
  const verdict = deps.limiter.check(deps.house.capabilityName, who.agentId, who.accountId, deps.limits);
  if (!verdict.ok) return refuse(refusal("limit", limited(verdict.reason, verdict.retryAfterSecs), verdict.limit));
  const callId = deps.limiter.record(deps.house.capabilityName, who.agentId, who.accountId);

  return deps.lock.run(who.agentId, async () => {
    const release = await deps.slots.acquire();
    const started = Date.now();
    let costUsd = 0;
    let outcome: Outcome;
    try {
      if (deps.stopping()) {
        const r = refusal("restarting", "The Guide is restarting. Ask again in a minute.");
        logRefusal("guide.refused", inv, r);
        outcome = failed(r.message);
      } else {
        const answer = await ask(deps.claude, deps.sessions, who.agentId, input.question, input.newSession, (usd) => {
          costUsd += usd;
          deps.limiter.addCost(callId, usd);
        });
        outcome = {
          status: "succeeded",
          output: guideOutput(answer.text, answer.cutShort, answer.turn, answer.newSession),
        };
      }
    } catch (err) {
      if (!(err instanceof GuideError)) log.error("guide.unexpected", { error: errorName(err) });
      outcome = failed(err instanceof GuideError ? err.userMessage : "The Guide hit an error. Try again in a minute.");
    } finally {
      release();
    }
    log.info("guide.answered", {
      invocation_id: inv.id,
      agent_id: who.agentId,
      status: outcome.status,
      ms: Date.now() - started,
      cost_usd: Number(costUsd.toFixed(5)),
    });
    await respond(deps.chakra, inv, outcome, deadline());
  });
}
