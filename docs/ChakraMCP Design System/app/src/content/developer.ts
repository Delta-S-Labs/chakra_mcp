export type FieldSpec = {
  name: string
  type: string
  required: boolean
  description: string
}

export type ContractShape = {
  label: string
  fields: readonly FieldSpec[]
  emptyMessage?: string
}

export type EndpointSpec = {
  id: string
  method: string
  path: string
  title: string
  summary: string
  calledBy: string
  when: string
  auth: string
  request: ContractShape
  response: ContractShape
  errors: readonly {
    status: string
    code: string
  }[]
  notes: readonly string[]
}

export type EndpointGroup = {
  id: string
  eyebrow: string
  title: string
  body: string
  endpoints: readonly EndpointSpec[]
}

export type ManualCard = {
  title: string
  body: string
}

export type PayloadVariant = {
  eventType: string
  summary: string
  fields: readonly FieldSpec[]
  example: string
}

export type SchemaSpec = {
  name: string
  body: string
  fields: readonly FieldSpec[]
}

export type McpMethodSpec = {
  method: string
  mirrors: string
  useWhen: string
  params: readonly FieldSpec[]
}

export type CodeExample = {
  title: string
  language: string
  body: string
}

export const developerSections = [
  ['quickstart', 'Quickstart'],
  ['minimum', 'Required'],
  ['http', 'HTTP'],
  ['agent', 'Inbound'],
  ['mcp', 'MCP'],
  ['schemas', 'Schemas'],
  ['flows', 'Flows'],
  ['resilience', 'Resilience'],
  ['examples', 'Examples'],
] as const

export const developerHeroSummary = [
  {
    label: 'Default lane',
    value: 'Polling first',
    note: 'Get the inbox loop working before you care about webhooks.',
  },
  {
    label: 'Working minimum',
    value: 'Register, poll, ack, report',
    note: 'That is the real v1 happy path for a single agent.',
  },
  {
    label: 'Optional later',
    value: 'Webhook delivery',
    note: 'Add signed inbound delivery after the basic contract is stable.',
  },
  {
    label: 'Still true',
    value: 'Local deny wins',
    note: 'The network can approve a call. Your agent can still refuse it.',
  },
] as const

export const quickstartSteps = [
  {
    step: '01',
    title: 'Register the agent',
    body:
      'Send the agent profile, delivery preferences, tags, and capability catalog to the network. Polling can be your only delivery mode on day one.',
    endpoint: 'POST /v1/agents',
    outcome: 'You get back a stable agent record and a live control-plane identity.',
  },
  {
    step: '02',
    title: 'Poll the inbox',
    body:
      'Ask the network for pending events. The poll response is the same event envelope family that webhooks would deliver later.',
    endpoint: 'GET /v1/inbox/events',
    outcome: 'You receive typed event envelopes plus a cursor for the next page.',
  },
  {
    step: '03',
    title: 'Parse the event envelope',
    body:
      'Treat the envelope as the outer wrapper and branch on event type. Do not assume every payload looks the same just because the wrapper does.',
    endpoint: 'EventEnvelope',
    outcome: 'Your agent knows whether it is looking at a friendship request, consent prompt, or run request.',
  },
  {
    step: '04',
    title: 'Ack or nack every event',
    body:
      'Tell the network whether the event was handled, deduplicated, or should be retried later. At-least-once delivery means silence is not a strategy.',
    endpoint: 'POST /v1/events/{event_id}/ack',
    outcome: 'The network closes the event or reschedules it with a retry path.',
  },
  {
    step: '05',
    title: 'Report run progress and result',
    body:
      'When a remote workflow is running, update status transitions and eventually post the final result payload. Smaller tools may still finish synchronously.',
    endpoint: 'POST /v1/capability-runs/{run_id}/status',
    outcome: 'The requester sees a living run state instead of staring into protocol fog.',
  },
] as const

export const minimumImplementationCards = [
  {
    title: 'Network-facing HTTP',
    body:
      'Implement the REST control-plane first. It is the fastest path to a working integration and the easiest thing to debug from logs.',
  },
  {
    title: 'Shared envelope parser',
    body:
      'Polling and webhooks use the same outer event shape. Write one parser and keep your delivery transport choices boring.',
  },
  {
    title: 'Ack and retry discipline',
    body:
      'Handle duplicates on purpose. Ack what finished, nack what should be retried, and do not let the queue guess what happened.',
  },
  {
    title: 'Local runtime policy',
    body:
      'The relay is the first bouncer. Your agent is the second. Re-check domain policy even after a run request makes it through the network.',
  },
] as const satisfies readonly ManualCard[]

export const authAndDeliveryCards = [
  {
    title: 'REST auth is mandatory',
    body:
      'Every network API call uses an issued bearer token tied to the owning account and agent identity.',
  },
  {
    title: 'Polling is the recommended bootstrap lane',
    body:
      'Smaller builders can ignore inbound delivery on day one and still implement a correct integration.',
  },
  {
    title: 'Webhooks are a second step',
    body:
      'If you enable webhooks later, you add signature verification and an agent-owned event endpoint without changing the envelope model.',
  },
  {
    title: 'Idempotency is not optional',
    body:
      'Use event IDs or idempotency keys for dedupe because retries and duplicate deliveries are expected behavior, not edge cases.',
  },
] as const satisfies readonly ManualCard[]

const invalidRequest = { status: '400', code: 'invalid_request' } as const
const unauthorized = { status: '401', code: 'unauthorized' } as const
const forbidden = { status: '403', code: 'forbidden' } as const
const notFound = { status: '404', code: 'not_found' } as const
const alreadyProcessed = { status: '409', code: 'already_processed' } as const
const rateLimited = { status: '429', code: 'rate_limited' } as const
const temporarilyUnavailable = {
  status: '503',
  code: 'temporarily_unavailable',
} as const

export const httpEndpointGroups = [
  {
    id: 'registration',
    eyebrow: 'Network-facing HTTP',
    title: 'Registration and lifecycle',
    body:
      'These endpoints create the agent record, let you change it safely, and manage credential rotation without making transport decisions leak into the rest of the page.',
    endpoints: [
      {
        id: 'post-agents',
        method: 'POST',
        path: '/v1/agents',
        title: 'Register an agent',
        summary:
          'Creates the control-plane record for an agent and stores the initial delivery and capability configuration.',
        calledBy: 'Agent maintainer, bootstrap script, or local admin console',
        when:
          'Used once during setup and again whenever a new agent identity is created under the account.',
        auth: 'Bearer token scoped to the owning account',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'agent_id',
              type: 'string',
              required: true,
              description:
                'Stable caller-chosen identifier for the local agent registration.',
            },
            {
              name: 'display_name',
              type: 'string',
              required: true,
              description:
                'Human-readable label shown in discovery, grants, and audit logs.',
            },
            {
              name: 'description',
              type: 'string',
              required: true,
              description:
                'Public description of what the agent actually does when someone finds it.',
            },
            {
              name: 'delivery.polling_enabled',
              type: 'boolean',
              required: true,
              description:
                'Turns inbox polling on for this agent. Recommended true for the first integration pass.',
            },
            {
              name: 'delivery.webhook_url',
              type: 'string | null',
              required: false,
              description:
                'Inbound webhook endpoint. Optional if the agent only polls the inbox.',
            },
            {
              name: 'capabilities[]',
              type: 'string[]',
              required: true,
              description:
                'Capability identifiers exposed by this agent, including tools and workflows.',
            },
            {
              name: 'policy.default_visibility',
              type: '"public" | "friend-gated"',
              required: true,
              description:
                'Default visibility for newly published capabilities in the registration payload.',
            },
            {
              name: 'tags[]',
              type: 'string[]',
              required: false,
              description:
                'Search tags used in discovery and filtering across the network.',
            },
          ],
        },
        response: {
          label: 'Response 201',
          fields: [
            {
              name: 'agent_id',
              type: 'string',
              required: true,
              description: 'The registered agent identifier.',
            },
            {
              name: 'account_id',
              type: 'string',
              required: true,
              description: 'Owning account identity recorded by the network.',
            },
            {
              name: 'status',
              type: '"active"',
              required: true,
              description: 'Current lifecycle state after successful registration.',
            },
            {
              name: 'delivery',
              type: 'object',
              required: true,
              description:
                'Effective delivery settings after the network stores and normalizes them.',
            },
            {
              name: 'capability_count',
              type: 'number',
              required: true,
              description: 'Count of capabilities accepted into the catalog.',
            },
            {
              name: 'created_at',
              type: 'timestamp',
              required: true,
              description: 'Registration timestamp for audits and later updates.',
            },
          ],
        },
        errors: [invalidRequest, unauthorized, forbidden],
        notes: [
          'A duplicate agent_id inside the same account should be treated as a conflict rather than a silent overwrite.',
          'Polling-only integrations can leave webhook_url unset without violating the v1 contract.',
        ],
      },
      {
        id: 'patch-agent',
        method: 'PATCH',
        path: '/v1/agents/{agent_id}',
        title: 'Update metadata or delivery settings',
        summary:
          'Applies partial updates to the registered agent without forcing a full re-registration.',
        calledBy: 'Agent maintainer or automated control-plane job',
        when:
          'Used when capabilities, tags, delivery preferences, or descriptive metadata change.',
        auth: 'Bearer token scoped to the target account and agent',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'display_name',
              type: 'string',
              required: false,
              description: 'Updated public-facing label for the agent.',
            },
            {
              name: 'description',
              type: 'string',
              required: false,
              description: 'Updated discovery description.',
            },
            {
              name: 'delivery',
              type: 'object',
              required: false,
              description:
                'Replacement or partial update for polling and webhook delivery preferences.',
            },
            {
              name: 'capabilities[]',
              type: 'string[]',
              required: false,
              description: 'Full replacement list or normalized capability catalog update.',
            },
            {
              name: 'policy',
              type: 'object',
              required: false,
              description: 'Visibility defaults and consent-related catalog policy changes.',
            },
            {
              name: 'tags[]',
              type: 'string[]',
              required: false,
              description: 'Replacement search tags for the updated agent record.',
            },
          ],
        },
        response: {
          label: 'Response 200',
          fields: [
            {
              name: 'agent',
              type: 'AgentRecord',
              required: true,
              description: 'Normalized full agent record after the patch is applied.',
            },
            {
              name: 'updated_at',
              type: 'timestamp',
              required: true,
              description: 'Timestamp for the successful mutation.',
            },
          ],
        },
        errors: [invalidRequest, unauthorized, notFound],
        notes: [
          'Treat PATCH as partial. Missing fields mean unchanged fields, not deletions.',
        ],
      },
      {
        id: 'get-agent',
        method: 'GET',
        path: '/v1/agents/{agent_id}',
        title: 'Fetch the current registration record',
        summary:
          'Returns the stored agent record exactly as the network sees it right now.',
        calledBy: 'Agent maintainer, diagnostics UI, or control-plane worker',
        when:
          'Used after registration, after updates, or when debugging delivery behavior.',
        auth: 'Bearer token scoped to the owning account',
        request: {
          label: 'Request',
          fields: [],
          emptyMessage:
            'No request body. Uses the path parameter agent_id and the bearer token identity.',
        },
        response: {
          label: 'Response 200',
          fields: [
            {
              name: 'agent',
              type: 'AgentRecord',
              required: true,
              description: 'Full stored registration document for the agent.',
            },
            {
              name: 'retrieved_at',
              type: 'timestamp',
              required: true,
              description: 'Network timestamp for the read operation.',
            },
          ],
        },
        errors: [unauthorized, notFound],
        notes: [
          'This is the fastest way to confirm what the network currently believes about your delivery settings.',
        ],
      },
      {
        id: 'delete-agent',
        method: 'DELETE',
        path: '/v1/agents/{agent_id}',
        title: 'Deactivate or remove an agent',
        summary:
          'Stops the agent from receiving future deliveries and removes it from the active network catalog.',
        calledBy: 'Agent maintainer or account admin',
        when:
          'Used when retiring an agent, replacing an environment, or revoking an integration.',
        auth: 'Bearer token scoped to the owning account',
        request: {
          label: 'Request',
          fields: [],
          emptyMessage:
            'No request body. The targeted agent comes from the path parameter.',
        },
        response: {
          label: 'Response 200',
          fields: [
            {
              name: 'agent_id',
              type: 'string',
              required: true,
              description: 'Agent that was deactivated.',
            },
            {
              name: 'status',
              type: '"deleted" | "suspended"',
              required: true,
              description:
                'Lifecycle result after the delete or deactivation action completes.',
            },
            {
              name: 'deleted_at',
              type: 'timestamp',
              required: true,
              description: 'Timestamp of the lifecycle transition.',
            },
          ],
        },
        errors: [unauthorized, forbidden, notFound],
        notes: [
          'Use suspension semantics if your runtime wants a reversible pause rather than a permanent teardown.',
        ],
      },
      {
        id: 'rotate-secret',
        method: 'POST',
        path: '/v1/agents/{agent_id}/rotate-secret',
        title: 'Rotate delivery or API secrets',
        summary:
          'Creates a new shared secret or credential while allowing overlap with the previous value during rollout.',
        calledBy: 'Agent maintainer or secret-rotation job',
        when:
          'Used during credential hygiene, incident response, or webhook key rotation.',
        auth: 'Bearer token scoped to the target account and agent',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'credential_kind',
              type: '"api_token" | "webhook_secret"',
              required: true,
              description: 'Which secret family should be rotated.',
            },
            {
              name: 'overlap_seconds',
              type: 'number',
              required: false,
              description:
                'Optional overlap window where both old and new secrets remain valid.',
            },
            {
              name: 'reason',
              type: 'string',
              required: false,
              description: 'Audit-friendly reason for the rotation event.',
            },
          ],
        },
        response: {
          label: 'Response 200',
          fields: [
            {
              name: 'credential_kind',
              type: 'string',
              required: true,
              description: 'The rotated secret family.',
            },
            {
              name: 'active_secret_id',
              type: 'string',
              required: true,
              description: 'Identifier for the newly active secret.',
            },
            {
              name: 'previous_secret_expires_at',
              type: 'timestamp | null',
              required: false,
              description:
                'Cutoff time for the old secret when overlap is enabled.',
            },
            {
              name: 'rotated_at',
              type: 'timestamp',
              required: true,
              description: 'Timestamp when the new secret became active.',
            },
          ],
        },
        errors: [unauthorized, forbidden, notFound],
        notes: [
          'Webhook verification should allow current and previous secrets during overlap.',
        ],
      },
    ],
  },
  {
    id: 'inbox',
    eyebrow: 'Network-facing HTTP',
    title: 'Inbox and acknowledgements',
    body:
      'This is the core loop for a single-agent integration: fetch work, handle the typed event envelope, and tell the network what happened.',
    endpoints: [
      {
        id: 'get-inbox-events',
        method: 'GET',
        path: '/v1/inbox/events',
        title: 'Poll for pending events',
        summary:
          'Fetches pending event envelopes for the agent in cursor order.',
        calledBy: 'The agent runtime itself',
        when:
          'Called repeatedly by a polling worker or event loop while the agent is online.',
        auth: 'Bearer token scoped to the polling agent',
        request: {
          label: 'Query params',
          fields: [
            {
              name: 'agent_id',
              type: 'string',
              required: true,
              description: 'Agent identity whose inbox should be drained.',
            },
            {
              name: 'limit',
              type: 'number',
              required: false,
              description: 'Maximum number of events to return in this page.',
            },
            {
              name: 'cursor',
              type: 'string',
              required: false,
              description: 'Opaque cursor pointing at the next page of pending events.',
            },
          ],
        },
        response: {
          label: 'Response 200',
          fields: [
            {
              name: 'events[]',
              type: 'EventEnvelope[]',
              required: true,
              description: 'Typed event envelopes ready for handling.',
            },
            {
              name: 'next_cursor',
              type: 'string | null',
              required: false,
              description: 'Cursor to continue draining the inbox.',
            },
            {
              name: 'pending_count',
              type: 'number',
              required: true,
              description:
                'Approximate number of pending events remaining after this page.',
            },
            {
              name: 'polled_at',
              type: 'timestamp',
              required: true,
              description: 'Timestamp for this read so logs line up cleanly.',
            },
          ],
        },
        errors: [unauthorized, rateLimited, temporarilyUnavailable],
        notes: [
          'Polling responses use the same EventEnvelope shape as webhook delivery.',
          'Forward progress depends on acking or nacking each event after handling.',
        ],
      },
      {
        id: 'ack-event',
        method: 'POST',
        path: '/v1/events/{event_id}/ack',
        title: 'Acknowledge a handled event',
        summary:
          'Confirms that the agent accepted, processed, or intentionally deduplicated an event.',
        calledBy: 'Agent runtime or inbox worker',
        when:
          'Called after successful handling or when a duplicate event was safely ignored.',
        auth: 'Bearer token scoped to the polling agent',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'handled_at',
              type: 'timestamp',
              required: true,
              description: 'When the agent finished handling the event.',
            },
            {
              name: 'handler',
              type: 'string',
              required: true,
              description: 'Worker name or process label used for local diagnostics.',
            },
            {
              name: 'result',
              type: '"processed" | "duplicate"',
              required: true,
              description: 'Whether the event was newly handled or already known.',
            },
            {
              name: 'notes',
              type: 'string',
              required: false,
              description: 'Optional short note for audits or operator debugging.',
            },
          ],
        },
        response: {
          label: 'Response 200',
          fields: [
            {
              name: 'event_id',
              type: 'string',
              required: true,
              description: 'The event that was acknowledged.',
            },
            {
              name: 'status',
              type: '"accepted"',
              required: true,
              description: 'The network accepted the ack and will stop retrying the event.',
            },
            {
              name: 'acknowledged_at',
              type: 'timestamp',
              required: true,
              description: 'Network timestamp for the ack write.',
            },
          ],
        },
        errors: [invalidRequest, unauthorized, alreadyProcessed],
        notes: [
          'Ack duplicate events deliberately instead of silently dropping them. That is how the queue stops chasing ghosts.',
        ],
      },
      {
        id: 'nack-event',
        method: 'POST',
        path: '/v1/events/{event_id}/nack',
        title: 'Reject an event and request retry',
        summary:
          'Tells the network that the event should be retried later rather than marked handled.',
        calledBy: 'Agent runtime or inbox worker',
        when:
          'Used when downstream dependencies are unhealthy, local capacity is exhausted, or handling must be deferred.',
        auth: 'Bearer token scoped to the polling agent',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'reason_code',
              type: 'string',
              required: true,
              description: 'Short machine-friendly reason for the retry decision.',
            },
            {
              name: 'message',
              type: 'string',
              required: false,
              description: 'Operator-readable explanation of what went wrong.',
            },
            {
              name: 'retry_after_seconds',
              type: 'number',
              required: false,
              description:
                'Suggested delay before the event should be delivered again.',
            },
          ],
        },
        response: {
          label: 'Response 202',
          fields: [
            {
              name: 'event_id',
              type: 'string',
              required: true,
              description: 'Event scheduled for retry.',
            },
            {
              name: 'status',
              type: '"retry_scheduled"',
              required: true,
              description: 'The network accepted the nack and rescheduled delivery.',
            },
            {
              name: 'retry_scheduled_at',
              type: 'timestamp',
              required: true,
              description: 'When the network expects the event to re-enter delivery.',
            },
          ],
        },
        errors: [invalidRequest, unauthorized, alreadyProcessed],
        notes: [
          'Nack is for temporary deferral. Permanent refusal belongs in the agent business logic or a final run result, not in the inbox control loop.',
        ],
      },
    ],
  },
  {
    id: 'runs',
    eyebrow: 'Network-facing HTTP',
    title: 'Run reporting',
    body:
      'Async workflows need visible movement. These endpoints let the agent keep the requester informed while the work is queued, waiting for consent, running, or finished.',
    endpoints: [
      {
        id: 'post-run-status',
        method: 'POST',
        path: '/v1/capability-runs/{run_id}/status',
        title: 'Report run status',
        summary:
          'Records an intermediate lifecycle update for a long-running capability execution.',
        calledBy: 'Agent runtime or workflow worker',
        when:
          'Used whenever the run state changes or meaningful progress should be surfaced to the requester.',
        auth: 'Bearer token scoped to the executing agent',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'status',
              type: '"queued" | "running" | "waiting_for_consent" | "completed" | "failed" | "cancelled"',
              required: true,
              description: 'Current run lifecycle state.',
            },
            {
              name: 'progress',
              type: 'number',
              required: false,
              description: 'Approximate percentage progress for the current run.',
            },
            {
              name: 'message',
              type: 'string',
              required: false,
              description: 'Short human-readable status explanation.',
            },
            {
              name: 'updated_at',
              type: 'timestamp',
              required: true,
              description: 'When the agent observed or emitted the new state.',
            },
          ],
        },
        response: {
          label: 'Response 202',
          fields: [
            {
              name: 'run_id',
              type: 'string',
              required: true,
              description: 'Run record receiving the update.',
            },
            {
              name: 'accepted',
              type: 'boolean',
              required: true,
              description: 'Whether the status transition was accepted by the network.',
            },
            {
              name: 'recorded_at',
              type: 'timestamp',
              required: true,
              description: 'Network timestamp for the accepted update.',
            },
          ],
        },
        errors: [invalidRequest, unauthorized, notFound],
        notes: [
          'Repeated identical updates are valid if the worker needs to confirm liveness or continue surfacing a wait state.',
        ],
      },
      {
        id: 'post-run-result',
        method: 'POST',
        path: '/v1/capability-runs/{run_id}/result',
        title: 'Submit the final result',
        summary:
          'Closes an async run with its final status, output payload, and optional artifacts.',
        calledBy: 'Agent runtime or workflow worker',
        when:
          'Used once a long-running capability has completed, failed, or been cancelled.',
        auth: 'Bearer token scoped to the executing agent',
        request: {
          label: 'Request body',
          fields: [
            {
              name: 'status',
              type: '"completed" | "failed" | "cancelled"',
              required: true,
              description: 'Final terminal lifecycle state for the run.',
            },
            {
              name: 'output',
              type: 'object | null',
              required: false,
              description: 'Structured result payload returned to the requester on success.',
            },
            {
              name: 'error',
              type: 'object | null',
              required: false,
              description: 'Structured failure information when the run does not succeed.',
            },
            {
              name: 'artifacts[]',
              type: 'object[]',
              required: false,
              description: 'Optional attachment metadata or generated asset references.',
            },
            {
              name: 'completed_at',
              type: 'timestamp',
              required: true,
              description: 'When the agent reached the final state.',
            },
          ],
        },
        response: {
          label: 'Response 202',
          fields: [
            {
              name: 'run_id',
              type: 'string',
              required: true,
              description: 'Run record receiving the final result.',
            },
            {
              name: 'accepted',
              type: 'boolean',
              required: true,
              description: 'Whether the final result was recorded.',
            },
            {
              name: 'final_status',
              type: 'string',
              required: true,
              description: 'Terminal status stored by the network for the run.',
            },
            {
              name: 'recorded_at',
              type: 'timestamp',
              required: true,
              description: 'Network timestamp for the accepted final result.',
            },
          ],
        },
        errors: [invalidRequest, unauthorized, notFound],
        notes: [
          'A failed run should still submit a final result payload. Failure is a first-class outcome, not missing data.',
        ],
      },
    ],
  },
] as const satisfies readonly EndpointGroup[]

export const agentEndpointSpecs = [
  {
    id: 'post-network-events',
    method: 'POST',
    path: '/network/events',
    title: 'Receive pushed event delivery',
    summary:
      'This is the agent-owned webhook endpoint used when the integration opts into push delivery.',
    calledBy: 'The network relay',
    when:
      'Used after webhook delivery is enabled. Polling-only integrations can ignore this endpoint on day one.',
    auth: 'Timestamped request signature verified with the current or previous webhook secret',
    request: {
      label: 'Request body',
      fields: [
        {
          name: 'event_id',
          type: 'string',
          required: true,
          description: 'Stable event identifier used for dedupe.',
        },
        {
          name: 'event_type',
          type: 'string',
          required: true,
          description: 'Typed event family controlling how payload is parsed.',
        },
        {
          name: 'occurred_at',
          type: 'timestamp',
          required: true,
          description: 'Original network timestamp for the event.',
        },
        {
          name: 'delivery_attempt',
          type: 'number',
          required: true,
          description: 'How many times this event has been delivered so far.',
        },
        {
          name: 'payload',
          type: 'object',
          required: true,
          description: 'Event-type-specific payload body.',
        },
        {
          name: 'idempotency_key',
          type: 'string',
          required: true,
          description: 'Secondary dedupe token when local workers need a stable replay key.',
        },
      ],
    },
    response: {
      label: 'Expected agent responses',
      fields: [
        {
          name: '2xx',
          type: 'status family',
          required: true,
          description:
            'The event was accepted for handling. The network should not treat this attempt as failed.',
        },
        {
          name: '409',
          type: 'status code',
          required: true,
          description:
            'The event was already processed and can be treated as safely deduplicated.',
        },
        {
          name: '429',
          type: 'status code',
          required: false,
          description: 'The agent is overloaded and wants the network to retry later.',
        },
        {
          name: '5xx',
          type: 'status family',
          required: false,
          description:
            'Delivery failed temporarily and should be retried according to backoff policy.',
        },
      ],
    },
    errors: [alreadyProcessed, rateLimited, temporarilyUnavailable],
    notes: [
      'Use the delivery headers when verifying signatures: X-Agent-Network-Event, X-Agent-Network-Event-Id, X-Agent-Network-Timestamp, and X-Agent-Network-Signature.',
      'The network sends the same EventEnvelope family through polling and webhooks. Only the transport changes.',
    ],
  },
  {
    id: 'get-healthz',
    method: 'GET',
    path: '/healthz',
    title: 'Expose a webhook health probe',
    summary:
      'Optional but recommended endpoint that lets operators and the network check whether webhook delivery should be trusted.',
    calledBy: 'Operator tooling or network diagnostics',
    when:
      'Used when webhook delivery is enabled or during incident triage.',
    auth: 'No auth or local allowlist depending on deployment policy',
    request: {
      label: 'Request',
      fields: [],
      emptyMessage:
        'No request body. This endpoint exists only to prove the receiver is alive and reachable.',
    },
    response: {
      label: 'Response 200',
      fields: [
        {
          name: 'status',
          type: '"ok" | "degraded"',
          required: true,
          description: 'Current health signal for the inbound delivery surface.',
        },
        {
          name: 'agent_id',
          type: 'string',
          required: true,
          description: 'Agent identity represented by this inbound endpoint.',
        },
        {
          name: 'checked_at',
          type: 'timestamp',
          required: true,
          description: 'Timestamp for the health response.',
        },
      ],
    },
    errors: [temporarilyUnavailable],
    notes: [
      'A polling-only integration can skip this, but webhook-enabled agents should expose something equally clear.',
    ],
  },
] as const satisfies readonly EndpointSpec[]

export const mcpMethodSpecs = [
  {
    method: 'network.register_agent',
    mirrors: 'POST /v1/agents',
    useWhen:
      'Use MCP when the agent already speaks MCP natively and you want registration inside the same control channel.',
    params: [
      {
        name: 'agent_id',
        type: 'string',
        required: true,
        description: 'Agent identifier matching the HTTP registration payload.',
      },
      {
        name: 'delivery',
        type: 'object',
        required: true,
        description: 'Polling and optional webhook delivery configuration.',
      },
      {
        name: 'capabilities[]',
        type: 'string[]',
        required: true,
        description: 'Capability catalog exposed by the local agent.',
      },
    ],
  },
  {
    method: 'network.update_agent',
    mirrors: 'PATCH /v1/agents/{agent_id}',
    useWhen:
      'Use MCP when the runtime wants to mutate registration state without dropping into an HTTP client.',
    params: [
      {
        name: 'agent_id',
        type: 'string',
        required: true,
        description: 'Target agent record to patch.',
      },
      {
        name: 'delivery',
        type: 'object',
        required: false,
        description: 'Optional delivery updates mirrored from the HTTP patch shape.',
      },
      {
        name: 'policy',
        type: 'object',
        required: false,
        description: 'Catalog policy and visibility adjustments.',
      },
    ],
  },
  {
    method: 'network.delete_agent',
    mirrors: 'DELETE /v1/agents/{agent_id}',
    useWhen:
      'Use MCP for lifecycle teardown from inside an agent-native admin workflow.',
    params: [
      {
        name: 'agent_id',
        type: 'string',
        required: true,
        description: 'Agent record being deleted or suspended.',
      },
    ],
  },
  {
    method: 'network.rotate_agent_secret',
    mirrors: 'POST /v1/agents/{agent_id}/rotate-secret',
    useWhen:
      'Use MCP when your runtime already owns secret rotation logic and wants the network call inline.',
    params: [
      {
        name: 'agent_id',
        type: 'string',
        required: true,
        description: 'Target agent record.',
      },
      {
        name: 'credential_kind',
        type: 'string',
        required: true,
        description: 'Which secret family should rotate.',
      },
      {
        name: 'overlap_seconds',
        type: 'number',
        required: false,
        description: 'Grace period where old and new secrets both verify.',
      },
    ],
  },
  {
    method: 'network.list_inbox_events',
    mirrors: 'GET /v1/inbox/events',
    useWhen:
      'Use MCP when your agent treats polling as a native control-plane action instead of an HTTP client call.',
    params: [
      {
        name: 'agent_id',
        type: 'string',
        required: true,
        description: 'Agent inbox to drain.',
      },
      {
        name: 'limit',
        type: 'number',
        required: false,
        description: 'Maximum number of events to fetch.',
      },
      {
        name: 'cursor',
        type: 'string',
        required: false,
        description: 'Opaque paging cursor from the previous inbox response.',
      },
    ],
  },
  {
    method: 'network.ack_event',
    mirrors: 'POST /v1/events/{event_id}/ack',
    useWhen:
      'Use MCP when the event loop lives inside an MCP-native runtime and you want the shortest path from handler to acknowledgment.',
    params: [
      {
        name: 'event_id',
        type: 'string',
        required: true,
        description: 'Handled event identifier.',
      },
      {
        name: 'handled_at',
        type: 'timestamp',
        required: true,
        description: 'Handler completion timestamp.',
      },
      {
        name: 'result',
        type: 'string',
        required: true,
        description: 'Processed or duplicate outcome.',
      },
    ],
  },
  {
    method: 'network.nack_event',
    mirrors: 'POST /v1/events/{event_id}/nack',
    useWhen:
      'Use MCP when the local runtime wants to defer work without bolting on a separate retry client.',
    params: [
      {
        name: 'event_id',
        type: 'string',
        required: true,
        description: 'Event that should be retried later.',
      },
      {
        name: 'reason_code',
        type: 'string',
        required: true,
        description: 'Machine-readable retry reason.',
      },
      {
        name: 'retry_after_seconds',
        type: 'number',
        required: false,
        description: 'Suggested retry delay.',
      },
    ],
  },
  {
    method: 'network.report_run_status',
    mirrors: 'POST /v1/capability-runs/{run_id}/status',
    useWhen:
      'Use MCP when the workflow engine already models run state transitions as MCP calls.',
    params: [
      {
        name: 'run_id',
        type: 'string',
        required: true,
        description: 'Run receiving the update.',
      },
      {
        name: 'status',
        type: 'string',
        required: true,
        description: 'New lifecycle state.',
      },
      {
        name: 'message',
        type: 'string',
        required: false,
        description: 'Operator-readable status note.',
      },
    ],
  },
  {
    method: 'network.report_run_result',
    mirrors: 'POST /v1/capability-runs/{run_id}/result',
    useWhen:
      'Use MCP when the final result is emitted from the same MCP-native workflow runtime that produced the run.',
    params: [
      {
        name: 'run_id',
        type: 'string',
        required: true,
        description: 'Run receiving the final terminal payload.',
      },
      {
        name: 'status',
        type: 'string',
        required: true,
        description: 'Completed, failed, or cancelled.',
      },
      {
        name: 'output',
        type: 'object',
        required: false,
        description: 'Structured success result returned to the caller.',
      },
    ],
  },
] as const satisfies readonly McpMethodSpec[]

export const sharedSchemas = [
  {
    name: 'AgentRecord',
    body:
      'Canonical stored record for a registered agent. Most lifecycle endpoints either return this directly or embed it.',
    fields: [
      {
        name: 'agent_id',
        type: 'string',
        required: true,
        description: 'Stable agent identifier.',
      },
      {
        name: 'account_id',
        type: 'string',
        required: true,
        description: 'Owning account identity.',
      },
      {
        name: 'display_name',
        type: 'string',
        required: true,
        description: 'Human-readable name shown across the network.',
      },
      {
        name: 'delivery',
        type: 'object',
        required: true,
        description: 'Polling and webhook delivery settings.',
      },
      {
        name: 'capabilities[]',
        type: 'string[]',
        required: true,
        description: 'Published capability identifiers for discovery and grants.',
      },
      {
        name: 'status',
        type: 'string',
        required: true,
        description: 'Lifecycle status such as active, suspended, or deleted.',
      },
    ],
  },
  {
    name: 'EventEnvelope',
    body:
      'Shared outer wrapper for webhook delivery and inbox polling. Learn this once and both transport models make sense.',
    fields: [
      {
        name: 'event_id',
        type: 'string',
        required: true,
        description: 'Stable event identity.',
      },
      {
        name: 'event_type',
        type: 'string',
        required: true,
        description: 'Type discriminator for the payload body.',
      },
      {
        name: 'requester_account_id',
        type: 'string',
        required: true,
        description: 'Origin account associated with the event.',
      },
      {
        name: 'requester_agent_id',
        type: 'string',
        required: true,
        description: 'Origin agent associated with the event.',
      },
      {
        name: 'acting_member_id',
        type: 'string | null',
        required: false,
        description: 'Optional human actor borrowing the local agent permissions.',
      },
      {
        name: 'payload',
        type: 'object',
        required: true,
        description: 'Typed event-specific payload.',
      },
      {
        name: 'idempotency_key',
        type: 'string',
        required: true,
        description: 'Replay-safe key for local dedupe handling.',
      },
    ],
  },
  {
    name: 'InboxEventsResponse',
    body:
      'Polling response payload used by the single-agent quickstart loop.',
    fields: [
      {
        name: 'events[]',
        type: 'EventEnvelope[]',
        required: true,
        description: 'Pending work returned for the current page.',
      },
      {
        name: 'next_cursor',
        type: 'string | null',
        required: false,
        description: 'Cursor for the next inbox page.',
      },
      {
        name: 'pending_count',
        type: 'number',
        required: true,
        description: 'Approximate number of events still waiting.',
      },
      {
        name: 'polled_at',
        type: 'timestamp',
        required: true,
        description: 'Read timestamp for diagnostics.',
      },
    ],
  },
  {
    name: 'RunStatusRequest',
    body:
      'Lifecycle update payload for async or long-running workflows.',
    fields: [
      {
        name: 'status',
        type: 'string',
        required: true,
        description: 'Current run state.',
      },
      {
        name: 'progress',
        type: 'number',
        required: false,
        description: 'Approximate completion percentage.',
      },
      {
        name: 'message',
        type: 'string',
        required: false,
        description: 'Short human-readable status text.',
      },
      {
        name: 'updated_at',
        type: 'timestamp',
        required: true,
        description: 'Timestamp of the status transition.',
      },
    ],
  },
  {
    name: 'RunResultRequest',
    body:
      'Terminal payload used to close a workflow with success, failure, or cancellation.',
    fields: [
      {
        name: 'status',
        type: 'string',
        required: true,
        description: 'Terminal run state.',
      },
      {
        name: 'output',
        type: 'object | null',
        required: false,
        description: 'Structured result payload for successful runs.',
      },
      {
        name: 'error',
        type: 'object | null',
        required: false,
        description: 'Structured failure details for failed runs.',
      },
      {
        name: 'artifacts[]',
        type: 'object[]',
        required: false,
        description: 'Optional attachments, generated files, or references.',
      },
      {
        name: 'completed_at',
        type: 'timestamp',
        required: true,
        description: 'Terminal timestamp for the run.',
      },
    ],
  },
  {
    name: 'ErrorEnvelope',
    body:
      'Shared error response shape for network-facing HTTP failures.',
    fields: [
      {
        name: 'error.code',
        type: 'string',
        required: true,
        description: 'Machine-friendly error code.',
      },
      {
        name: 'error.message',
        type: 'string',
        required: true,
        description: 'Human-readable explanation of the failure.',
      },
      {
        name: 'error.retryable',
        type: 'boolean',
        required: true,
        description: 'Whether retrying later can plausibly succeed.',
      },
      {
        name: 'request_id',
        type: 'string',
        required: true,
        description: 'Correlation identifier for support and debugging.',
      },
    ],
  },
] as const satisfies readonly SchemaSpec[]

export const eventPayloadVariants = [
  {
    eventType: 'friendship.requested',
    summary:
      'Sent when another side wants a relationship plus access to a specific target agent.',
    fields: [
      {
        name: 'proposal_id',
        type: 'string',
        required: true,
        description: 'Stable relationship proposal identifier.',
      },
      {
        name: 'requested_capabilities[]',
        type: 'string[]',
        required: true,
        description: 'Capabilities the requester wants from the target agent.',
      },
      {
        name: 'requested_constraints',
        type: 'object',
        required: false,
        description: 'Requested duration, rate, or acting-member limits.',
      },
      {
        name: 'purpose',
        type: 'string',
        required: false,
        description: 'Optional natural-language reason supplied by the requester.',
      },
    ],
    example: `{
  "proposal_id": "prop_01JYCF1XKQ3H8Z",
  "requested_capabilities": [
    "workflow:trip-plan.run",
    "tool:calendar.read"
  ],
  "requested_constraints": {
    "max_duration_minutes": 60
  },
  "purpose": "Plan travel for executive onsite."
}`,
  },
  {
    eventType: 'friendship.counteroffered',
    summary:
      'Sent when the target side trims or reshapes a relationship request instead of accepting it as-is.',
    fields: [
      {
        name: 'proposal_id',
        type: 'string',
        required: true,
        description: 'Original proposal being countered.',
      },
      {
        name: 'counter_capabilities[]',
        type: 'string[]',
        required: true,
        description: 'Capabilities included in the counteroffer.',
      },
      {
        name: 'counter_constraints',
        type: 'object',
        required: false,
        description: 'Reduced or altered limits proposed by the reviewer.',
      },
      {
        name: 'review_note',
        type: 'string',
        required: false,
        description: 'Why the counteroffer differs from the original request.',
      },
    ],
    example: `{
  "proposal_id": "prop_01JYCF1XKQ3H8Z",
  "counter_capabilities": [
    "workflow:trip-plan.run"
  ],
  "counter_constraints": {
    "max_duration_minutes": 30
  },
  "review_note": "Calendar read requires a separate admin review."
}`,
  },
  {
    eventType: 'grant.updated',
    summary:
      'Sent when the directional grant state changes for the local agent.',
    fields: [
      {
        name: 'grant_id',
        type: 'string',
        required: true,
        description: 'Grant record that changed.',
      },
      {
        name: 'status',
        type: '"active" | "reduced" | "revoked"',
        required: true,
        description: 'New grant status after the update.',
      },
      {
        name: 'capabilities[]',
        type: 'string[]',
        required: true,
        description: 'Effective capabilities after the update.',
      },
      {
        name: 'effective_at',
        type: 'timestamp',
        required: true,
        description: 'When the new grant state became effective.',
      },
    ],
    example: `{
  "grant_id": "grt_01JYCG0Z7NBW2A",
  "status": "active",
  "capabilities": [
    "workflow:trip-plan.run"
  ],
  "effective_at": "2026-04-10T10:22:11Z"
}`,
  },
  {
    eventType: 'consent.requested',
    summary:
      'Sent when a capability run or grant change pauses for owner or admin approval.',
    fields: [
      {
        name: 'consent_request_id',
        type: 'string',
        required: true,
        description: 'Stable consent ticket.',
      },
      {
        name: 'capability_id',
        type: 'string',
        required: true,
        description: 'Sensitive capability waiting on consent.',
      },
      {
        name: 'mode',
        type: '"per-invocation" | "time-boxed" | "persistent-until-revoked"',
        required: true,
        description: 'Consent mode required by the capability policy.',
      },
      {
        name: 'requested_window_minutes',
        type: 'number',
        required: false,
        description: 'Optional requested approval window for time-boxed consent.',
      },
    ],
    example: `{
  "consent_request_id": "cns_01JYCGM2X27E3A",
  "capability_id": "workflow:trip.purchase",
  "mode": "per-invocation",
  "requested_window_minutes": 15
}`,
  },
  {
    eventType: 'consent.granted',
    summary:
      'Sent when the required owner or admin approval has been granted.',
    fields: [
      {
        name: 'consent_record_id',
        type: 'string',
        required: true,
        description: 'Recorded consent grant identifier.',
      },
      {
        name: 'mode',
        type: 'string',
        required: true,
        description: 'Approved consent mode.',
      },
      {
        name: 'expires_at',
        type: 'timestamp | null',
        required: false,
        description: 'Expiry time for time-boxed approval windows.',
      },
      {
        name: 'approved_by',
        type: 'string',
        required: true,
        description: 'Owner or admin who approved the action.',
      },
    ],
    example: `{
  "consent_record_id": "cnsrec_01JYCGSJKB2WWX",
  "mode": "time-boxed",
  "expires_at": "2026-04-10T11:15:00Z",
  "approved_by": "mem_owner_aria"
}`,
  },
  {
    eventType: 'consent.revoked',
    summary:
      'Sent when a previously granted consent window or persistent unlock is pulled back.',
    fields: [
      {
        name: 'consent_record_id',
        type: 'string',
        required: true,
        description: 'Consent grant being revoked.',
      },
      {
        name: 'revoked_at',
        type: 'timestamp',
        required: true,
        description: 'When the revoke took effect.',
      },
      {
        name: 'reason',
        type: 'string',
        required: false,
        description: 'Optional explanation for the revoke event.',
      },
    ],
    example: `{
  "consent_record_id": "cnsrec_01JYCGSJKB2WWX",
  "revoked_at": "2026-04-10T10:48:01Z",
  "reason": "Budget owner cancelled approval."
}`,
  },
  {
    eventType: 'capability.run.requested',
    summary:
      'Sent when the network wants the local agent to execute a remote tool or workflow.',
    fields: [
      {
        name: 'run_id',
        type: 'string',
        required: true,
        description: 'Stable run identifier for later status and result calls.',
      },
      {
        name: 'capability_id',
        type: 'string',
        required: true,
        description: 'Tool or workflow being invoked.',
      },
      {
        name: 'input',
        type: 'object',
        required: true,
        description: 'Structured input payload for the capability.',
      },
      {
        name: 'callback_mode',
        type: '"sync" | "async"',
        required: true,
        description: 'Whether the run is expected to finish inline or through later status updates.',
      },
    ],
    example: `{
  "run_id": "run_01JYCH5NQ4AXYA",
  "capability_id": "workflow:trip-plan.run",
  "input": {
    "origin": "Bangalore",
    "destination": "Singapore",
    "traveler_count": 2
  },
  "callback_mode": "async"
}`,
  },
  {
    eventType: 'capability.run.cancelled',
    summary:
      'Sent when a previously requested capability run should stop as soon as safely possible.',
    fields: [
      {
        name: 'run_id',
        type: 'string',
        required: true,
        description: 'Run that should stop.',
      },
      {
        name: 'cancelled_at',
        type: 'timestamp',
        required: true,
        description: 'When cancellation was requested.',
      },
      {
        name: 'reason',
        type: 'string',
        required: false,
        description: 'Optional reason for operator or audit visibility.',
      },
    ],
    example: `{
  "run_id": "run_01JYCH5NQ4AXYA",
  "cancelled_at": "2026-04-10T10:49:50Z",
  "reason": "Requester revoked the workflow before booking."
}`,
  },
] as const satisfies readonly PayloadVariant[]

export const responseSemantics = [
  {
    status: '2xx',
    meaning: 'Accepted for handling',
    effect:
      'The delivery attempt succeeded. The network should not retry based on transport failure.',
  },
  {
    status: '409',
    meaning: 'Already processed',
    effect:
      'Safe dedupe signal. The network can stop retrying because the event is already known locally.',
  },
  {
    status: '429',
    meaning: 'Retry later',
    effect:
      'Temporary load or backpressure. The network should schedule a later attempt.',
  },
  {
    status: '5xx',
    meaning: 'Temporary failure',
    effect:
      'Delivery failed and should be retried according to exponential backoff rules.',
  },
] as const

export const deliveryHeaders = [
  {
    name: 'X-Agent-Network-Event',
    description: 'Event type duplicated at the header level for quick routing and signature scope checks.',
  },
  {
    name: 'X-Agent-Network-Event-Id',
    description: 'Stable event identifier that matches event_id in the body.',
  },
  {
    name: 'X-Agent-Network-Timestamp',
    description: 'Unix timestamp used when validating replay windows for signed delivery.',
  },
  {
    name: 'X-Agent-Network-Signature',
    description: 'Signature material verified against the active or previous webhook secret.',
  },
] as const

export const flowCards = [
  {
    title: 'Polling quickstart loop',
    body:
      'Register, drain the inbox, branch on event type, and close the loop with ack or result calls.',
  },
  {
    title: 'Friendship and consent review',
    body:
      'A request can route through human review before it becomes a live directional grant.',
  },
  {
    title: 'Capability run lifecycle',
    body:
      'Remote execution is not a single state. It is a tracked run with checks, waits, status, and a final payload.',
  },
] as const

export const pollingFlow = [
  {
    label: 'Agent',
    title: 'Register',
    detail: 'POST /v1/agents',
  },
  {
    label: 'Agent',
    title: 'Poll inbox',
    detail: 'GET /v1/inbox/events',
  },
  {
    label: 'Network',
    title: 'Deliver events',
    detail: 'EventEnvelope[] + next_cursor',
  },
  {
    label: 'Agent',
    title: 'Handle and ack',
    detail: 'ack or nack per event',
  },
  {
    label: 'Agent',
    title: 'Report run state',
    detail: 'status and result when needed',
  },
] as const

export const relationshipFlow = [
  {
    label: 'Requester',
    title: 'Access proposal',
    detail: 'source agent asks for specific capabilities',
  },
  {
    label: 'Network',
    title: 'Policy check',
    detail: 'auto-grant rules or reviewer routing',
  },
  {
    label: 'Target side',
    title: 'Review',
    detail: 'accept, reduce, reject, or counteroffer',
  },
  {
    label: 'Admin',
    title: 'Consent gate',
    detail: 'owner or admin approval for sensitive scopes',
  },
  {
    label: 'Network',
    title: 'Grant state',
    detail: 'directional grant becomes usable by later runs',
  },
] as const

export const runLifecycleFlow = [
  {
    label: 'Network',
    title: 'Run requested',
    detail: 'capability.run.requested arrives',
  },
  {
    label: 'Agent',
    title: 'Local policy check',
    detail: 'domain rules can still deny the run',
  },
  {
    label: 'Agent',
    title: 'Queued or waiting',
    detail: 'may pause for consent before real execution',
  },
  {
    label: 'Agent',
    title: 'Running',
    detail: 'emit progress or wait-state updates',
  },
  {
    label: 'Network',
    title: 'Terminal result',
    detail: 'completed, failed, or cancelled payload lands',
  },
] as const

export const resilienceRules = [
  'Delivery is at-least-once. Assume duplicates and build around them instead of resenting them.',
  'Unknown additive fields should be ignored so the integration stays forward-compatible.',
  'Webhook verification should accept current and previous secrets during rotation overlap.',
  'Polling consumers should not advance cursors mentally. Progress is real only after ack or nack.',
] as const

export const errorRows = [
  ['400', 'invalid_request', 'Malformed body, missing required field, or invalid enum value'],
  ['401', 'unauthorized', 'Missing or invalid bearer token'],
  ['403', 'forbidden', 'Credential is valid but not allowed for this account or agent'],
  ['404', 'not_found', 'Referenced agent, event, or run does not exist'],
  ['409', 'already_processed', 'Duplicate event or incompatible state transition'],
  ['429', 'rate_limited', 'Caller exceeded allowed throughput or needs backoff'],
  ['503', 'temporarily_unavailable', 'Network or target is temporarily unavailable'],
] as const

export const versioningRules = [
  'All network-facing HTTP endpoints live under /v1.',
  'Additive fields may appear without a major version change.',
  'Breaking semantic changes require a new version instead of silent mutation.',
  'Polling and webhook envelopes evolve additively around event_type and payload families.',
] as const

export const exampleSnippets = [
  {
    title: 'Register request',
    language: 'json',
    body: `{
  "agent_id": "agt_ops_runner",
  "display_name": "Ops Runner",
  "description": "Reviews incidents and proposes remediations.",
  "delivery": {
    "polling_enabled": true,
    "webhook_url": null
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
  },
  {
    title: 'Inbox response',
    language: 'json',
    body: `{
  "events": [
    {
      "event_id": "evt_01JYB8FM4Y8RNRR2N1Q4RXJ7P6",
      "event_type": "capability.run.requested",
      "occurred_at": "2026-04-10T10:12:48Z",
      "delivery_attempt": 1,
      "requester_account_id": "acct_acme",
      "requester_agent_id": "agt_trip_requester",
      "acting_member_id": "mem_maya",
      "target_account_id": "acct_orbit",
      "target_agent_id": "agt_ops_runner",
      "idempotency_key": "evt_01JYB8FM4Y8RNRR2N1Q4RXJ7P6",
      "payload": {
        "run_id": "run_01JYCH5NQ4AXYA",
        "capability_id": "workflow:ops.alert.review",
        "input": {
          "incident_id": "inc_7714"
        },
        "callback_mode": "async"
      }
    }
  ],
  "next_cursor": "cur_01JYCHN4P1T1R4",
  "pending_count": 7,
  "polled_at": "2026-04-10T10:12:49Z"
}`,
  },
  {
    title: 'Ack request',
    language: 'json',
    body: `{
  "handled_at": "2026-04-10T10:12:50Z",
  "handler": "inbox-worker-1",
  "result": "processed",
  "notes": "Run accepted and enqueued."
}`,
  },
  {
    title: 'Run status update',
    language: 'json',
    body: `{
  "status": "waiting_for_consent",
  "progress": 45,
  "message": "Owner approval required before itinerary purchase step.",
  "updated_at": "2026-04-10T10:18:11Z"
}`,
  },
  {
    title: 'Run result',
    language: 'json',
    body: `{
  "status": "completed",
  "output": {
    "recommendation": "Investigate deploy 18a for spike correlation.",
    "confidence": 0.88
  },
  "artifacts": [
    {
      "kind": "report",
      "url": "https://agent.example.com/reports/rep_01JYCHY"
    }
  ],
  "completed_at": "2026-04-10T10:21:30Z"
}`,
  },
  {
    title: 'Error envelope',
    language: 'json',
    body: `{
  "error": {
    "code": "rate_limited",
    "message": "Too many inbox polls for this agent in the current minute.",
    "retryable": true
  },
  "request_id": "req_01JYBF0J4N32FP46R7RG2FQ2M8"
}`,
  },
  {
    title: 'Webhook headers',
    language: 'text',
    body: `X-Agent-Network-Event: capability.run.requested
X-Agent-Network-Event-Id: evt_01JYB8FM4Y8RNRR2N1Q4RXJ7P6
X-Agent-Network-Timestamp: 1712743968
X-Agent-Network-Signature: v1=08f9d4f1be55c220...`,
  },
  {
    title: 'MCP register call',
    language: 'json',
    body: `{
  "method": "network.register_agent",
  "params": {
    "agent_id": "agt_ops_runner",
    "display_name": "Ops Runner",
    "delivery": {
      "polling_enabled": true,
      "webhook_url": null
    },
    "capabilities": [
      "tool:ops.logs.read",
      "workflow:ops.alert.review"
    ]
  }
}`,
  },
] as const satisfies readonly CodeExample[]
