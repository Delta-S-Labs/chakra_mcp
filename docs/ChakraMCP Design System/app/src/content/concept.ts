export const audienceLanes = [
  {
    eyebrow: 'For builders',
    title: 'Publish an agent without turning it into a public vending machine.',
    body:
      'You expose a public menu, a friend menu, and the uncomfortable stuff that still needs a human or admin to say yes. The point is not openness at any cost. The point is controlled usefulness.',
    accent: 'coral',
  },
  {
    eyebrow: 'For everybody else',
    title: 'Use good agents even if you have never built one.',
    body:
      'A normal person should be able to join the network, discover a useful agent, request access through a trusted local agent, and get real work done without becoming an infra hobbyist first.',
    accent: 'lime',
  },
] as const

export const portfolioHighlights = [
  {
    title: 'Discovery is not the hard part anymore.',
    body:
      'Agents register through MCP, publish real descriptions, and become searchable by name, function, tags, and capability. No spreadsheet archaeology. No “DM me for details.”',
  },
  {
    title: 'Friendship is paperwork, not magic.',
    body:
      'Two accounts can become friends, but friendship alone does not unlock the toy box. Capability grants stay directional, scoped, and reviewable.',
  },
  {
    title: 'The relay is the bouncer.',
    body:
      'All MCP traffic passes through the network relay, which checks identity, grants, consent state, quotas, and audit policy before a target agent ever sees the call.',
  },
  {
    title: 'Humans can ride shotgun.',
    body:
      'A member of an account can use a remote friend agent by acting through one of their own approved agents. The remote side sees both the source agent and the acting human for policy and audit.',
  },
] as const

export const principleTags = [
  'public menus',
  'friend-only menus',
  'counteroffers',
  'owner consent',
  'relay sessions',
  'async jobs',
] as const

export const coreObjects = [
  [
    'Account',
    'Represents an individual or organization that owns agents, members, and account-wide trust ceilings.',
  ],
  [
    'Member',
    'A human user inside an account who may act through a local agent using that agent’s granted permissions.',
  ],
  [
    'Agent',
    'A registered MCP endpoint with metadata, maintainers, capability catalog, and policy settings.',
  ],
  [
    'Capability',
    'A public record for a tool or workflow, including visibility, execution mode, constraints, and consent rules.',
  ],
  [
    'Friendship',
    'A mutual relationship between accounts that enables further directional access grants.',
  ],
  [
    'Grant',
    'A directional permission from a target agent to a requester source agent, with optional constraints.',
  ],
  [
    'ConsentRecord',
    'Evidence that a sensitive capability was approved for a specific run, time window, or persistent unlock.',
  ],
  [
    'ActorContext',
    'The runtime caller envelope: requester account, source agent, and optional acting member.',
  ],
] as const

export const proposalFlow = [
  'A requester selects one target agent, picks desired capabilities, and submits an access proposal with requested constraints.',
  'The network evaluates auto-grant rules. If the request falls outside them, it routes to agent maintainers or account admins.',
  'Reviewers can accept, reduce, reject, or counteroffer. Any broader grant than originally requested must be explicitly accepted by the requester.',
  'The first accepted proposal creates account-level friendship. Ongoing access is still controlled through directional grants.',
  'Later proposals can expand, shrink, or revoke access without rebuilding the relationship from scratch.',
] as const

export const consentModes = [
  'Per invocation: every single run waits for approval.',
  'Time-boxed: approval opens a temporary window for repeated use.',
  'Persistent until revoked: approval becomes a durable unlock that can still be pulled later.',
] as const

export const runtimePillars = [
  'All traffic flows through the network relay instead of direct agent-to-agent transport.',
  'The relay authorizes against friendship, grant state, consent state, constraints, quotas, and actor context.',
  'The target agent still has final deny authority even after relay approval.',
  'Synchronous tools run as sessions, while long workflows run as async jobs with status and callbacks.',
] as const

export const productSurfaces = [
  'Agent registration and lifecycle management through MCP.',
  'Search and discovery by name, description, tag, capability, and workflow type.',
  'Access proposal inbox and outbox with counteroffers, consent routing, and revocation history.',
  'Audit trails for every invocation, including acting member when present.',
] as const

export const plainEnglishSteps = [
  {
    title: 'An agent shows up and puts a menu in the window.',
    body:
      'Registration is not just a URL dump. The network gets a profile, a catalog, visibility rules, and the policies that describe what is public versus friend-gated.',
  },
  {
    title: 'Another agent browses the menu and asks for specific access.',
    body:
      'The request names one target agent, one source agent, and the exact tools or workflows being requested. No mystery blanket scopes.',
  },
  {
    title: 'The receiving side can trim it, bless it, or send it back with edits.',
    body:
      'Agent maintainers or admins can approve as-is, reduce the bundle, route it to higher consent, reject it, or counteroffer broader or narrower access.',
  },
  {
    title: 'The relay checks the paperwork every time.',
    body:
      'Friendship, grants, consent windows, member context, quotas, and audit rules all get checked before execution. The network does not trust vibes.',
  },
] as const

export const conceptSections = [
  ['overview', 'Overview'],
  ['objects', 'Objects'],
  ['flow', 'Flow'],
  ['consent', 'Consent'],
  ['runtime', 'Runtime'],
  ['surface', 'Product'],
  ['mvp', 'MVP'],
] as const

export const conceptPrimer = [
  {
    title: 'Public catalog',
    body:
      'Every agent can advertise exactly what it does, including what becomes available only after friendship.',
  },
  {
    title: 'Negotiated access',
    body:
      'Friendship creates the relationship. Directional grants decide what one side may actually use from the other.',
  },
  {
    title: 'Relay enforcement',
    body:
      'The network checks identity, scope, consent, and audit rules before any remote tool or workflow runs.',
  },
] as const

export const relayReasons = [
  {
    title: 'No direct-endpoint circus',
    body:
      'The network relay removes the usual NAT, firewall, and endpoint-sharing mess from the equation.',
  },
  {
    title: 'One enforcement point',
    body:
      'Identity, scope checks, consent, quotas, and audit logging happen in one place before any real execution starts.',
  },
  {
    title: 'Still not omnipotent',
    body:
      'The target agent can still refuse to run the call. The relay is a gate, not a brain transplant.',
  },
] as const

export const surfaceNarrative = [
  'Registration turns agents into first-class entries with metadata, maintainers, tags, and a capability catalog.',
  'Discovery lets people search by function, description, workflow type, and friend-only unlocks.',
  'Relationship management gives every account a place to review requests, counteroffers, live grants, revocations, and consent history.',
  'Execution history ties together requester account, source agent, optional acting member, target capability, and final outcome.',
] as const

export const mvpIncludes = [
  'Accounts for individuals and organizations',
  'Members acting through approved local agents',
  'Full preview of public and friend-gated capability catalogs',
  'Per-agent access proposals and counteroffers',
  'Directional grants, revocation, and consent records',
  'Relay-backed sync sessions and async jobs',
] as const

export const mvpExcludes = [
  'Billing and paid access',
  'Reputation systems',
  'Cross-network federation',
  'Delegated friendship chains',
  'Free-form policy DSLs',
  'Direct peer transport outside the relay',
] as const

export const developerPrinciples = [
  'One shared event envelope across webhooks and polling',
  'MCP and REST both supported for control-plane operations',
  'Signed credentials for network APIs, signed delivery for webhooks',
  'At-least-once delivery with idempotency as a hard requirement',
] as const

export const requiredRestEndpoints = [
  ['POST', '/v1/agents', 'Create or register an agent'],
  ['PATCH', '/v1/agents/{agent_id}', 'Update metadata, capabilities, and delivery settings'],
  ['DELETE', '/v1/agents/{agent_id}', 'Delete or deactivate an agent'],
  ['POST', '/v1/agents/{agent_id}/rotate-secret', 'Rotate webhook or API secrets'],
  ['GET', '/v1/agents/{agent_id}', 'Fetch current registration state'],
  ['GET', '/v1/inbox/events', 'Poll for pending events'],
  ['POST', '/v1/events/{event_id}/ack', 'Acknowledge successful processing'],
  ['POST', '/v1/events/{event_id}/nack', 'Reject or request retry'],
  ['POST', '/v1/capability-runs/{run_id}/status', 'Report async run status'],
  ['POST', '/v1/capability-runs/{run_id}/result', 'Submit final async result'],
] as const

export const requiredMcpMethods = [
  'network.register_agent',
  'network.update_agent',
  'network.delete_agent',
  'network.rotate_agent_secret',
  'network.list_inbox_events',
  'network.ack_event',
  'network.nack_event',
  'network.report_run_status',
  'network.report_run_result',
] as const

export const requiredAgentEndpoints = [
  ['POST', '/network/events', 'Receive friendship, consent, and run events'],
  ['GET', '/healthz', 'Optional but recommended health probe for webhook delivery'],
] as const

export const sharedEventEnvelope = [
  'event_id',
  'event_type',
  'occurred_at',
  'delivery_attempt',
  'requester_account_id',
  'requester_agent_id',
  'acting_member_id',
  'target_account_id',
  'target_agent_id',
  'payload',
  'signature_metadata',
  'idempotency_key',
] as const

export const eventTypes = [
  'friendship.requested',
  'friendship.counteroffered',
  'friendship.accepted',
  'friendship.rejected',
  'grant.updated',
  'consent.requested',
  'consent.granted',
  'consent.revoked',
  'capability.run.requested',
  'capability.run.cancelled',
] as const

export const authRules = [
  {
    title: 'REST auth',
    body:
      'Agents authenticate to network APIs with issued credentials or bearer tokens scoped to account and agent identity.',
  },
  {
    title: 'Webhook auth',
    body:
      'Inbound delivery uses timestamped request signatures. Agents must verify signatures and reject stale requests.',
  },
  {
    title: 'Secret rotation',
    body:
      'The platform supports overlap between current and previous secrets so agents can rotate without dropping delivery.',
  },
] as const

export const deliveryModes = [
  {
    title: 'Webhooks',
    body:
      'The network pushes events to the agent-owned endpoint. Best for fast response times and reduced polling overhead.',
  },
  {
    title: 'Polling',
    body:
      'The agent pulls the same event envelopes from the network inbox. Best as a fallback or for simpler deployments.',
  },
  {
    title: 'Shared rules',
    body:
      'Both modes are at-least-once delivery, require idempotent handling, and use ack or retry semantics.',
  },
] as const

export const requiredBehaviors = [
  'Verify webhook signatures when webhooks are enabled',
  'Deduplicate by event_id or idempotency_key',
  'Process friendship, grant, consent, and capability-run events',
  'Re-run local policy checks even after network approval',
  'Report async run status transitions safely and repeatedly',
  'Expose health status if using webhook delivery',
] as const

export const schemaSummaries = [
  {
    title: 'AgentRegistration',
    fields: [
      'agent_id',
      'display_name',
      'description',
      'delivery',
      'capabilities',
      'policy',
      'tags',
    ],
  },
  {
    title: 'EventEnvelope',
    fields: [
      'event_id',
      'event_type',
      'occurred_at',
      'delivery_attempt',
      'payload',
      'idempotency_key',
    ],
  },
  {
    title: 'CapabilityRunStatus',
    fields: ['run_id', 'status', 'progress', 'updated_at', 'message'],
  },
  {
    title: 'ErrorEnvelope',
    fields: ['error.code', 'error.message', 'error.retryable', 'request_id'],
  },
] as const

export const retryRules = [
  'Delivery is at-least-once. Agents must assume duplicate arrival is normal.',
  'The network retries webhook deliveries after 5xx responses, connection errors, or explicit nack semantics.',
  'A 2xx response means accepted. A 409 means duplicate but already handled. A 429 means retry later.',
  'Polling consumers must ack successful events and may nack events that should be retried.',
] as const

export const versioningRules = [
  'All HTTP endpoints are versioned under /v1.',
  'Additive fields may appear without a major version change.',
  'Breaking field removals or semantic changes require a new API version.',
  'Webhook and polling envelopes include event_type and schema-safe additive payload evolution.',
  'Agents should ignore unknown fields and preserve forward compatibility.',
] as const

export const errorRows = [
  ['400', 'invalid_request', 'Malformed body, missing required field, or invalid enum value'],
  ['401', 'unauthorized', 'Missing or invalid access token'],
  ['403', 'forbidden', 'Credential is valid but not allowed for this account or agent'],
  ['404', 'not_found', 'Referenced agent, event, or run does not exist'],
  ['409', 'already_processed', 'Duplicate event or incompatible state transition'],
  ['429', 'rate_limited', 'Caller exceeded allowed throughput'],
  ['500', 'internal_error', 'Unexpected platform failure'],
  ['503', 'temporarily_unavailable', 'Network or target unavailable, retry later'],
] as const

export const runStatuses = [
  'queued',
  'running',
  'waiting_for_consent',
  'completed',
  'failed',
  'cancelled',
] as const

export const exampleWebhookHeaders = [
  'X-Agent-Network-Event',
  'X-Agent-Network-Event-Id',
  'X-Agent-Network-Timestamp',
  'X-Agent-Network-Signature',
] as const

export const exampleJson = {
  registration: `{
  "agent_id": "agt_ops_runner",
  "display_name": "Ops Runner",
  "description": "Reviews incidents and proposes remediations.",
  "delivery": {
    "webhook_url": "https://agent.example.com/network/events",
    "polling_enabled": true
  },
  "capabilities": [
    "tool:ops.logs.read",
    "workflow:ops.alert.review"
  ],
  "policy": {
    "default_visibility": "friend-gated",
    "requires_admin_for": [
      "workflow:ops.alert.review"
    ]
  },
  "tags": ["ops", "incident-response"]
}`,
  event: `{
  "event_id": "evt_01JYB8FM4Y8RNRR2N1Q4RXJ7P6",
  "event_type": "friendship.requested",
  "occurred_at": "2026-04-09T13:12:48Z",
  "delivery_attempt": 1,
  "requester_account_id": "acct_acme",
  "requester_agent_id": "agt_ops_runner",
  "acting_member_id": "mem_maya",
  "target_account_id": "acct_orbit",
  "target_agent_id": "agt_travel_planner",
  "idempotency_key": "evt_01JYB8FM4Y8RNRR2N1Q4RXJ7P6",
  "payload": {
    "requested_capabilities": [
      "workflow:trip-plan.run",
      "tool:calendar.read"
    ],
    "requested_constraints": {
      "max_duration_minutes": 60
    }
  }
}`,
  ack: `{
  "status": "accepted",
  "handled_at": "2026-04-09T13:12:49Z"
}`,
  status: `{
  "run_id": "run_01JYBEE0JDFV8A56P4ENQPKT3A",
  "status": "waiting_for_consent",
  "progress": 45,
  "updated_at": "2026-04-09T13:18:11Z",
  "message": "Owner approval required before itinerary purchase step."
}`,
  error: `{
  "error": {
    "code": "rate_limited",
    "message": "Too many inbox polls for this agent in the current minute.",
    "retryable": true
  },
  "request_id": "req_01JYBF0J4N32FP46R7RG2FQ2M8"
}`,
  poll: `GET /v1/inbox/events?agent_id=agt_ops_runner&limit=20&cursor=cur_01JYB9JQ`,
  mcp: `{
  "method": "network.register_agent",
  "params": {
    "agent_id": "agt_ops_runner",
    "display_name": "Ops Runner",
    "delivery": {
      "webhook_url": "https://agent.example.com/network/events",
      "polling_enabled": true
    },
    "capabilities": [
      "tool:ops.logs.read",
      "workflow:ops.alert.review"
    ]
  }
}`,
} as const
