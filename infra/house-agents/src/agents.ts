// The house agents and their one capability each, as they should exist on
// the relay. `ensureAgents` creates or corrects them at startup.

import type { Agent, Capability, ChakraMCP } from "@chakramcp/sdk";
import { log } from "./log.js";

export interface CapabilitySpec {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  public_monthly_quota_per_agent: number;
}

export interface AgentSpec {
  slug: string;
  display_name: string;
  description: string;
  /** Directory tags, so searches and tag filters find it. */
  tags: string[];
  capability: CapabilitySpec;
}

export interface HouseAgent {
  id: string;
  slug: string;
  capabilityId: string;
  capabilityName: string;
}

export const PING_MESSAGE_MAX = 500;
export const QUESTION_MAX_CHARS = 4000;
export const GUIDE_INPUT_MAX_BYTES = 8 * 1024;

export const PING: AgentSpec = {
  slug: "ping",
  display_name: "Ping",
  description:
    "Answers every call straight away with a pong. Call it to check that your agent can invoke over ChakraMCP: no friendship needed.",
  tags: ["testing", "devtools"],
  capability: {
    name: "ping.echo",
    description: `Replies with pong and echoes \`message\` (up to ${PING_MESSAGE_MAX} characters).`,
    input_schema: {
      type: "object",
      properties: { message: { type: "string", maxLength: PING_MESSAGE_MAX } },
      additionalProperties: false,
    },
    output_schema: {
      type: "object",
      required: ["pong", "answered_at"],
      properties: {
        pong: { type: "boolean" },
        message: { type: "string" },
        answered_at: { type: "string", format: "date-time" },
      },
    },
    public_monthly_quota_per_agent: 1000,
  },
};

export const GUIDE: AgentSpec = {
  slug: "guide",
  display_name: "ChakraMCP Guide",
  description:
    "Ask it how to use ChakraMCP: agents, capabilities, friendships and grants, MCP, the CLI and SDKs, credits, self-hosting. Powered by Claude, and remembers your conversation.",
  tags: ["docs", "help", "support"],
  capability: {
    name: "guide.ask",
    description:
      `Answers a question about ChakraMCP from its documentation. The Guide keeps one conversation per calling agent; send \`new_session: true\` to start over. Limits: 10 questions an hour and 50 a day per agent, one at a time.`,
    input_schema: {
      type: "object",
      required: ["question"],
      properties: {
        question: { type: "string", minLength: 1, maxLength: QUESTION_MAX_CHARS },
        new_session: { type: "boolean", description: "Start a new conversation." },
      },
      additionalProperties: false,
    },
    output_schema: {
      type: "object",
      required: ["answer"],
      properties: {
        answer: { type: "string", description: "Markdown." },
        cut_short: { type: "boolean" },
        conversation: {
          type: "object",
          properties: { turn: { type: "integer" }, new_session: { type: "boolean" } },
        },
      },
    },
    public_monthly_quota_per_agent: 300,
  },
};

/** JSON with sorted keys, to compare schemas that went through jsonb. */
export function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) =>
    v && typeof v === "object" && !Array.isArray(v)
      ? Object.fromEntries(Object.entries(v as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)))
      : v,
  );
}

export async function resolveAccount(chakra: ChakraMCP, slug: string): Promise<string> {
  const me = await chakra.me();
  const membership = me.memberships.find((m) => m.slug === slug);
  if (!membership) throw new Error(`the API key's user isn't a member of account "${slug}"`);
  return membership.account_id;
}

function sameTags(have: string[], want: string[]): boolean {
  return have.length === want.length && want.every((t) => have.includes(t));
}

async function ensureAgent(chakra: ChakraMCP, accountId: string, spec: AgentSpec, mine: Agent[]): Promise<HouseAgent> {
  let agent = mine.find((a) => a.account_id === accountId && a.slug === spec.slug);
  if (!agent) {
    agent = await chakra.agents.create({
      account_id: accountId,
      slug: spec.slug,
      display_name: spec.display_name,
      description: spec.description,
      visibility: "network",
      tags: spec.tags,
    });
    log.info("agent.created", { slug: spec.slug, agent_id: agent.id });
  } else if (
    agent.display_name !== spec.display_name ||
    agent.description !== spec.description ||
    agent.visibility !== "network" ||
    !sameTags(agent.tags ?? [], spec.tags)
  ) {
    agent = await chakra.agents.update(agent.id, {
      display_name: spec.display_name,
      description: spec.description,
      visibility: "network",
      tags: spec.tags,
    });
    log.info("agent.updated", { slug: spec.slug, agent_id: agent.id });
  }

  const want = spec.capability;
  const caps: Capability[] = await chakra.agents.capabilities.list(agent.id);
  let cap = caps.find((c) => c.name === want.name);
  const body = {
    description: want.description,
    input_schema: want.input_schema,
    output_schema: want.output_schema,
    visibility: "network" as const,
    public_invoke: true,
    public_monthly_quota_per_agent: want.public_monthly_quota_per_agent,
  };
  if (!cap) {
    cap = await chakra.agents.capabilities.create(agent.id, {
      name: want.name,
      ...body,
      semantics: "autonomous",
    });
    log.info("capability.created", { name: want.name, capability_id: cap.id });
  } else if (
    cap.description !== want.description ||
    canonical(cap.input_schema) !== canonical(want.input_schema) ||
    canonical(cap.output_schema) !== canonical(want.output_schema) ||
    cap.visibility !== "network" ||
    !cap.public_invoke ||
    cap.public_monthly_quota_per_agent !== want.public_monthly_quota_per_agent
  ) {
    cap = await chakra.relayRequest<Capability>(
      "PATCH",
      `/v1/agents/${encodeURIComponent(agent.id)}/capabilities/${encodeURIComponent(cap.id)}`,
      body,
    );
    log.info("capability.updated", { name: want.name, capability_id: cap.id });
  }
  return { id: agent.id, slug: spec.slug, capabilityId: cap.id, capabilityName: want.name };
}

/** Makes sure both house agents exist as specified; returns them in order. */
export async function ensureAgents(chakra: ChakraMCP, accountSlug: string): Promise<[HouseAgent, HouseAgent]> {
  const accountId = await resolveAccount(chakra, accountSlug);
  const mine = await chakra.agents.list();
  const ping = await ensureAgent(chakra, accountId, PING, mine);
  const guide = await ensureAgent(chakra, accountId, GUIDE, mine);
  return [ping, guide];
}
