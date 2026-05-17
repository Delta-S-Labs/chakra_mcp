/**
 * Reserved capability templates.
 *
 * These are capabilities the ChakraMCP protocol reserves a name for —
 * agents that publish the canonical name MUST conform to the canonical
 * schema. That lets peers discover and call a known capability without
 * per-agent integration code.
 *
 * Templates are exported as `const` literals (not classes) because
 * they're pure data — just JSON Schema. The `chakra.capabilities
 * .addTemplate()` helper looks one up by id and POSTs the equivalent
 * `capabilities.create()` body.
 *
 * Adding a new template: append an entry below, bump the SDK minor
 * version, and document it in /docs/agents under "Reserved capability
 * templates" — that page is the canonical source for peers (incl. ones
 * not using this SDK) and explains why the names are reserved.
 */

import type { CreateCapabilityRequest } from "./types";

/**
 * Body for `chakra.capabilities.create(agentId, body)`. Name +
 * description filled in; the user can override description but not
 * the schemas (defeats the point of a reserved name).
 */
export const MESSAGE_OWNER: CreateCapabilityRequest = {
  name: "message_owner",
  description:
    "Ping the human owner of this agent. Friend agents call this; " +
    "the owner sees the message and explicitly answers, acks, " +
    "ignores, or defers. Always human-in-the-loop.",
  // Issue #69 PR 2: published capabilities now carry a `semantics`
  // field, and `message_owner` is the canonical human-in-the-loop
  // case. The relay's gate at POST /v1/invocations/{id}/result will
  // reject worker-posted replies on HITL invocations unless they
  // carry `confirmed_by_human: true`. The TS SDK's default reply path
  // leaves that flag off — autonomous handlers that try to answer a
  // `message_owner` invocation get a 409 with code
  // `chk.policy.requires_human_confirmation` and must route the
  // invocation to a human instead (PR 4 plumbs the `humanHandler`
  // callback through `InboxClient.serve()`).
  semantics: "human_in_loop",
  input_schema: {
    type: "object",
    required: ["message"],
    properties: {
      message: {
        type: "string",
        minLength: 1,
        maxLength: 4000,
        description: "Message body. Plain text. Be concise.",
      },
      from_display_name: {
        type: "string",
        maxLength: 120,
        description: "Who's pinging. Falls back to calling agent's display_name.",
      },
      urgency: {
        type: "string",
        enum: ["low", "normal", "high"],
        default: "normal",
        description:
          "low = batch in next digest; normal = surface when owner next interacts; high = alert immediately.",
      },
      expects_reply: {
        type: "boolean",
        default: true,
        description: "If false, message is informational and an ack is enough.",
      },
      reply_by: {
        type: "string",
        format: "date-time",
        description: "Optional soft deadline. After this, handler may auto-defer.",
      },
    },
  },
  output_schema: {
    type: "object",
    required: ["status"],
    properties: {
      status: {
        type: "string",
        enum: ["replied", "acknowledged", "ignored", "deferred"],
      },
      reply: {
        type: "string",
        maxLength: 8000,
        description: "Owner's reply text. Present only when status='replied'.",
      },
      responded_at: {
        type: "string",
        format: "date-time",
      },
      defer_until: {
        type: "string",
        format: "date-time",
        description: "Present only when status='deferred'.",
      },
    },
  },
  visibility: "network",
};

/** Registry — id → body. Match the Python SDK's `CHAKRAMCP_TEMPLATES`. */
export const CHAKRAMCP_TEMPLATES: Readonly<Record<string, CreateCapabilityRequest>> = {
  message_owner: MESSAGE_OWNER,
};

/** Sorted list of reserved template ids. */
export function templateNames(): string[] {
  return Object.keys(CHAKRAMCP_TEMPLATES).sort();
}

/**
 * Look up a template by id. Throws if unknown.
 *
 * The returned object is shared — clone it before mutating.
 */
export function getTemplate(templateId: string): CreateCapabilityRequest {
  const t = CHAKRAMCP_TEMPLATES[templateId];
  if (!t) {
    const known = templateNames().join(", ") || "(none)";
    throw new Error(`unknown template id: "${templateId}". Known: ${known}`);
  }
  return t;
}
