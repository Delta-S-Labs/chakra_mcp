# Developer Page Redesign

## Overview

This redesign turns the developer page from a concept-heavy overview into a practical integration manual for a team connecting a single agent to the network. The primary audience is not a platform architect building the whole protocol stack from scratch. It is an implementer who wants to get one agent registered, connected, and correctly handling requests without reverse-engineering missing payloads or guessing runtime behavior.

The page should still preserve the project’s editorial tone and visual identity, but its first job is operational clarity. A builder should be able to land on the page, follow a polling-first quickstart, understand the exact request and response shapes for the network-facing endpoints, see what inbound payloads their agent will receive, and understand the main lifecycle flows through diagrams before they read deeper examples.

## Goals

- Make the developer page useful for a single-agent integrator on first read.
- Lead with a polling-first quickstart that covers the happy path end to end.
- Describe every required payload family for both network-facing and agent-facing contracts.
- Show request and response formats for every documented HTTP endpoint.
- Explain the three most important protocol flows with diagrams instead of prose alone.
- Improve visual hierarchy so the page reads like a field manual rather than a stack of identical cards.

## Non-Goals

- Redesign the portfolio or concept pages as part of this task.
- Define an exhaustive protocol RFC for every future feature.
- Add interactive API playgrounds or live request tooling.
- Replace the existing tone with sterile reference-doc language.

## Audience And Use Context

### Primary Audience

The primary reader is an engineer integrating one agent into the network. They want the shortest path to:

- registering an agent
- receiving requests
- polling for work
- acknowledging events correctly
- reporting run status and results
- understanding when webhooks matter later

### Secondary Audience

More advanced builders implementing deeper protocol surfaces should still be able to use the page as a reference, but the page should not optimize for them first.

## Page Strategy

The page should behave like an operator’s manual:

1. Show the fastest happy path first.
2. Explain only the minimum required surfaces.
3. Expand into full contract details.
4. Reinforce behavior with diagrams.
5. End with concrete examples and schema references.

The content hierarchy should make it obvious what is normative in v1 versus what is illustrative.

## Information Architecture

The page should be reorganized into five chapters.

### 1. Quickstart

This chapter replaces the current manifesto-like hero. It should present a polling-first integration path in five steps:

1. Register the agent.
2. Poll the inbox for events.
3. Parse the shared event envelope.
4. Ack or nack each event.
5. Report run status and final result.

The quickstart should include:

- one short framing paragraph
- one numbered visual rail or timeline
- one compact “what you need to implement” summary
- one small outcome panel showing the minimum surfaces required for a working agent

### 2. What You Must Implement

This chapter should define the minimum required integration surface as a checklist:

- network-facing HTTP endpoints
- optional network-facing MCP methods
- agent-owned inbound webhook endpoint when webhooks are enabled
- local runtime requirements such as idempotency, signature verification, and local policy re-checks

This section should be concise and visually scannable.

### 3. Contracts

This becomes the core of the page. Every documented contract item must answer:

- who calls it
- when it is used
- request payload
- response payload

The page should explicitly separate:

- network-facing endpoints
- agent-facing inbound endpoint
- shared schemas used by both

Every endpoint block should include:

- method and path
- purpose
- auth expectation
- request fields
- success response shape
- common error codes
- idempotency or retry notes where relevant

### 4. Flows

This chapter explains system behavior through diagrams, not just endpoint lists.

Required diagrams:

- polling quickstart loop
- friendship and consent review loop
- capability run lifecycle

These diagrams should be explanatory, not decorative. They should be readable on mobile as stacked sequences.

### 5. Examples

This chapter should contain concrete JSON and HTTP examples after the contract sections, not before them. It should reinforce the spec rather than substitute for it.

## Contract Scope

The redesigned page must document these network-facing HTTP endpoints:

- `POST /v1/agents`
- `PATCH /v1/agents/{agent_id}`
- `GET /v1/agents/{agent_id}`
- `DELETE /v1/agents/{agent_id}`
- `POST /v1/agents/{agent_id}/rotate-secret`
- `GET /v1/inbox/events`
- `POST /v1/events/{event_id}/ack`
- `POST /v1/events/{event_id}/nack`
- `POST /v1/capability-runs/{run_id}/status`
- `POST /v1/capability-runs/{run_id}/result`

The redesigned page must also document the mirrored network-facing MCP methods:

- `network.register_agent`
- `network.update_agent`
- `network.delete_agent`
- `network.rotate_agent_secret`
- `network.list_inbox_events`
- `network.ack_event`
- `network.nack_event`
- `network.report_run_status`
- `network.report_run_result`

The redesigned page must document these agent-facing endpoints:

- `POST /network/events`
- `GET /healthz` as optional and recommended for webhook delivery

The redesigned page must document these shared schema families:

- `AgentRegistrationRequest`
- `AgentRegistrationResponse`
- `AgentUpdateRequest`
- `AgentRecord`
- `InboxEventsResponse`
- `EventEnvelope`
- `AckRequest`
- `AckResponse`
- `NackRequest`
- `RunStatusRequest`
- `RunStatusResponse`
- `RunResultRequest`
- `RunResultResponse`
- `ErrorEnvelope`

## Payload Documentation Requirements

The current page’s biggest weakness is that it names endpoints without describing what implementers actually send or receive. The redesign must fix that directly.

### Network-Facing Payloads

For each network-facing HTTP endpoint, the page must show:

- request payload fields and short meanings
- whether fields are required or optional
- success response payload fields
- likely error envelope shape for failures

At minimum, the following payloads must be concretely shown:

- agent registration request and response
- agent update request and response
- polled inbox response including event list and pagination cursor
- ack request and ack response
- nack request and retry intent
- run status request and accepted response
- run result request and accepted response

### Agent-Facing Payloads

For `POST /network/events`, the page must show:

- delivery headers
- event envelope shape
- payload variants by `event_type`
- expected agent response semantics for `2xx`, `409`, `429`, and `5xx`

The page should make it explicit that the agent does not receive one generic opaque blob. It receives an event envelope with a typed payload that varies by event type.

### MCP Documentation

The MCP surface should be documented as a mirrored control-plane interface rather than as a second full reference manual. Each method should show:

- the method name
- the HTTP endpoint it mirrors
- the core params shape
- when a builder would choose MCP instead of HTTP

If a method’s payload semantics are identical to its HTTP counterpart, the page should avoid duplicating the full contract block and instead link or visually point to the shared schema.

### Event Payload Variants

The page should describe at least these payload families:

- friendship request payload
- friendship counteroffer payload
- grant updated payload
- consent requested payload
- consent granted payload
- consent revoked payload
- capability run requested payload
- capability run cancelled payload

Each payload family should include a representative field list and one example.

## Diagram Requirements

### Polling Quickstart Loop

This diagram should show:

- agent registration
- inbox polling
- receipt of event envelopes
- ack or nack branch
- optional status reporting
- final result submission

It should visually reinforce the polling-first recommendation for smaller integrations.

### Friendship And Consent Review Loop

This diagram should show:

- source agent or member intent
- access request creation
- target review
- admin or consent routing when required
- resulting grant state
- later use by capability runs

This diagram exists to connect the policy story to the runtime story.

### Capability Run Lifecycle

This diagram should show:

- inbound run request
- local policy validation
- optional consent wait
- queued or running state
- completion, failure, or cancellation
- status updates and final result

It should make sync and async behavior legible without turning into a dense systems chart.

## Visual Design Direction

The developer page should feel like a field manual, not marketing disguised as docs.

### Design Intent

- denser and sharper than the concept page
- stronger section contrast
- fewer soft repeated cards
- more ruled layouts, structured slabs, and visible hierarchy
- code and payload blocks that feel like reference tools, not decorative inserts

### Page Rhythm

The page should vary its presentation by content type:

- quickstart as numbered steps or a linear rail
- contracts as structured endpoint slabs
- schemas as compact field matrices
- diagrams as hand-built process maps
- examples as darker code wells with high contrast labels

This variation is necessary to avoid the current “same card repeated forever” feeling.

### Hero Treatment

The hero should stop behaving like a brand poster. It should become a quickstart header with:

- a shorter headline
- a brief one-paragraph framing note
- a visible five-step integration rail
- a compact summary panel on the right

### Tone

The writing should keep the project’s sly, slightly rogue voice, but the tone should not obscure implementation details. Clarity wins whenever tone and precision compete.

## Responsive Behavior

The redesigned page must remain strong on mobile.

- Sticky side navigation should collapse or transform into a simpler in-flow chapter index on small screens.
- Diagrams should stack into vertical sequences instead of shrinking into unreadable posters.
- Contract slabs should remain readable without horizontal scrolling except for code or literal JSON.
- Quickstart steps should preserve sequence and emphasis on smaller devices.

## Implementation Notes

The redesign should prefer reusable content structures instead of hardcoded one-off sections. The page is likely to need:

- a quickstart step data model
- an endpoint spec data model with request and response sections
- a schema field table model
- diagram components implemented in React and CSS
- example snippet blocks sourced from structured content

The content should distinguish clearly between:

- `Required in v1`
- `Example implementation`

That distinction should be visible in both copy and layout.

## Error Handling And Behavioral Notes

The page should explicitly document:

- at-least-once delivery expectations
- deduplication by `event_id` or `idempotency_key`
- retry semantics for webhook failures
- ack and nack expectations for polling
- forward-compatible parsing of additive fields
- the requirement that agents perform their own local policy checks even after network approval

## Validation

Implementation is complete when:

- the developer page leads with a polling-first quickstart
- every required endpoint includes request and response payload descriptions
- the inbound agent endpoint clearly documents payload variants and response semantics
- three diagrams explain the main flows
- the page visually reads as a field manual instead of a concept essay
- screenshots confirm the new hierarchy and readability on desktop and mobile
- `npm run lint` passes
- `npm run build` passes
