import type { Agent, FeaturedAgent, Invocation } from "./relay";

/** The capability the checklist suggests calling first. */
export const GUIDE_CAPABILITY = "guide.ask";

export type StepId = "create" | "connect" | "call" | "publish";

export interface GetStartedStep {
  id: StepId;
  done: boolean;
}

export interface GetStarted {
  steps: GetStartedStep[];
  doneCount: number;
  allDone: boolean;
  /** The ChakraMCP Guide, when the relay features it. */
  guide: { agentId: string; agentName: string; capabilityId: string } | null;
  /** The agent to call from: the first one created. */
  fromAgentId: string | null;
}

/**
 * The dashboard's "Get started" checklist, from what the page already loads:
 * 1. create an agent;
 * 2. connect it: it has polled its inbox, or made a call;
 * 3. call another agent (the house agents are there for this);
 * 4. publish a capability others can see.
 */
export function deriveGetStarted(input: {
  agents: Agent[];
  invocations: Invocation[];
  featured: FeaturedAgent[];
}): GetStarted {
  const { agents, invocations, featured } = input;
  const mine = agents.filter((a) => a.is_mine !== false);
  const called = invocations.some((i) => i.i_invoked);
  const steps: GetStartedStep[] = [
    { id: "create", done: mine.length > 0 },
    { id: "connect", done: called || mine.some((a) => Boolean(a.last_polled_at)) },
    { id: "call", done: called },
    { id: "publish", done: mine.some((a) => (a.network_capability_count ?? 0) > 0) },
  ];
  const doneCount = steps.filter((s) => s.done).length;

  let guide: GetStarted["guide"] = null;
  for (const a of featured) {
    if (!a.featured) continue;
    const cap = a.public_capabilities.find((c) => c.name === GUIDE_CAPABILITY);
    if (cap) {
      guide = { agentId: a.id, agentName: a.display_name, capabilityId: cap.id };
      break;
    }
  }

  const first = [...mine].sort((a, b) => a.created_at.localeCompare(b.created_at))[0];
  return { steps, doneCount, allDone: doneCount === steps.length, guide, fromAgentId: first?.id ?? null };
}

/** A CLI command that asks the Guide a question as `fromAgentId`. */
export function guideCommand(capabilityId: string, fromAgentId: string | null): string {
  return [
    "chakramcp invoke \\",
    `  --capability ${capabilityId} \\`,
    `  --as ${fromAgentId ?? "<your-agent-id>"} \\`,
    `  --input '{"question": "How do grants work?"}' \\`,
    "  --wait",
  ].join("\n");
}

/** A CLI command that gives `fromAgentId` a network capability from a template. */
export function publishCommand(fromAgentId: string | null): string {
  return [
    "chakramcp capabilities add \\",
    `  --agent ${fromAgentId ?? "<your-agent-id>"} \\`,
    "  --template message_owner",
  ].join("\n");
}
