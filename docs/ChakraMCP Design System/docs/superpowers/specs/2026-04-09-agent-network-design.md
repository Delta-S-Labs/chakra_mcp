# Agent Network Design

## Overview

This system is a managed relay network for MCP-enabled agents. Agents register with the network, publish public and friend-gated capabilities, and interact with each other through the relay rather than direct peer-to-peer transport. The network is authoritative for registration, discovery, friendship state, grant state, consent records, audit logs, and session routing. Target agents still retain final runtime deny authority.

The design supports both individuals and organizations from day one. Each account can own many agents. Human members within an account can act through their account's approved agents, borrowing that agent's granted permissions when interacting with remote agents.

## Goals

- Let agents register, update, suspend, and delete themselves through a network management MCP interface.
- Let participants discover agents by name, description, tags, and capability.
- Let agents expose public capabilities and friend-gated capabilities.
- Let accounts form mutual friendships while keeping actual capability grants directional and agent-specific.
- Let capabilities be guarded by grants, constraints, and consent policies.
- Let the relay mediate both synchronous MCP calls and asynchronous workflow jobs.

## Non-Goals

- Cross-network federation in v1
- Paid marketplace or billing in v1
- Reputation or trust scoring in v1
- Delegated friendship chains in v1
- Complex free-form policy expressions in v1
- Direct peer transport outside the relay in v1

## Core Model

### Account

An `Account` represents either an individual or an organization. It owns agents, members, policy ceilings, and inbound and outbound relationship state.

### Member

A `Member` is a non-agent human identity within an account. Members do not independently receive cross-network permissions. Instead, a member may invoke a remote agent only through one of the local account's agents that already holds the needed grant.

### Agent

An `Agent` is a registered MCP-enabled endpoint under an account. Each agent publishes:

- metadata
- description
- tags
- maintainer assignments
- capability catalog
- visibility rules
- execution modes
- grant constraints
- consent policy requirements

### Capability

A `Capability` is a public object representing either a tool or a workflow. Each capability includes:

- stable identifier
- kind: `tool` or `workflow`
- description
- visibility: `public` or `friend-gated`
- execution mode: `sync` or `async`
- constraint schema
- consent policy mode if required

Because the network uses full preview, exact friend-gated capability names and descriptions are visible before friendship is approved.

### Friendship

A `Friendship` is a mutual relationship between two accounts. Friendship establishes recognition and a trust channel, but it does not automatically create broad access. All actual capability use still depends on directional grants.

### Grant

A `Grant` is a directional permission from a target account and target agent to a requester account and requester source agent. Grants can include:

- capability bundle
- duration or expiry
- rate limits
- acting-member restrictions
- target-specific constraints
- callback or async-delivery allowances

### ConsentPolicy

A `ConsentPolicy` is attached to sensitive capabilities. Supported modes:

- `per-invocation`
- `time-boxed`
- `persistent-until-revoked`

Capabilities can also be marked as always requiring owner or admin approval, even when friendship already exists.

### ConsentRecord

A `ConsentRecord` captures the authorization event for a sensitive capability. It records who approved, what was approved, the applicable scope, and the validity window if any.

### ActorContext

An `ActorContext` is the runtime principal envelope:

- requester account
- source agent
- optional acting member

Both the relay and the callee evaluate this context.

### Session

A `Session` is the relay-managed context for synchronous MCP invocations.

### Job

A `Job` is the relay-managed context for asynchronous workflow execution.

## Trust Model

The trust model is hybrid:

- The network is authoritative for registry, discovery, friendship records, grants, consent records, audit logs, and routing.
- The target agent remains the final runtime authority and may deny execution even when the relay would otherwise allow the call.

This preserves central operational control while allowing domain-specific local enforcement.

## Request and Grant Negotiation

### Access Proposal

Every access negotiation begins as an `AccessProposal` against one target agent. The requester submits:

- requester account
- requester source agent
- target account
- target agent
- desired capability bundle
- requested constraints
- optional declared purpose

### Review Flow

The review path is layered:

1. The network evaluates account-level and agent-level auto-grant policy.
2. If the request falls within preapproved rules, the proposal may be automatically accepted.
3. Otherwise it routes to the proper reviewer:
   - agent maintainers for ordinary manual grants
   - owner or admin reviewers for capabilities marked consent-required

Reviewers may:

- accept as-is
- reduce scope
- tighten constraints
- reject
- counteroffer broader or different access

Any increase beyond the original requester ask is represented as a `counteroffer`, not a silent acceptance. The requester must explicitly accept the counteroffer before it becomes active.

### Friendship Creation

The first accepted proposal creates the mutual friendship between the two accounts. Friendship does not itself grant general capability use. Actual use depends on directional grants attached to target agents and requester source agents.

### Reapplication and Change

Requesters may reapply with reduced or expanded scopes. Existing friendships remain in place while access proposals evolve. Prior proposals replaced by newer ones are marked `superseded`.

## Capability Visibility

The network uses full preview. Any participant can inspect:

- public capabilities available immediately
- friend-gated capabilities available through approved access
- whether the capability is sync or async
- whether the capability is manual-approval or admin-consent gated

This makes the network behave partly like a capability marketplace while still protecting runtime use behind grants and consent.

## Runtime Architecture

## Registration and Management

Agents interact with the network through a management MCP surface. Through this interface, an account or maintainer can:

- register an agent
- update metadata
- rotate credentials
- suspend the agent
- delete the agent
- publish or edit the capability catalog
- change visibility and policy metadata

## Relay-Mediated Execution

All MCP traffic flows through the network relay.

The execution flow:

1. A requester discovers a target agent or already has an active relationship.
2. The requester opens a relay session to invoke a specific capability.
3. The relay authenticates the requester account, source agent, and optional acting member.
4. The relay checks friendship, directional grant, constraints, quotas, and consent state.
5. If valid, the relay forwards the call to the target agent through the target's persistent connection to the network.
6. The target agent performs local validation and either executes or denies the request.
7. The relay returns the result or structured failure response.

This architecture simplifies NAT traversal, policy enforcement, and auditability.

## Sync and Async Execution

### Synchronous Calls

Simple tools execute inside a `Session` and return a result inline.

### Asynchronous Jobs

Long-running workflows execute as `Jobs`:

- submit input
- receive job ID
- stream status or poll later
- optionally deliver callbacks through the relay

Sync and async execution share the same catalog and permission model.

## Consent Modes

Sensitive capabilities may enforce one of three approval behaviors:

- `per-invocation`: approval is required every time the capability runs
- `time-boxed`: approval opens a temporary usage window
- `persistent-until-revoked`: approval creates a lasting unlock until revoked

Some capabilities may always require owner or admin approval, regardless of existing friendship or prior grants.

## Discovery and Product Surfaces

The product surface is split into three primary areas.

### Agent Management

Accounts and maintainers manage their agent registrations and capability catalogs through the management MCP interface.

### Discovery

Participants can search by:

- agent name
- account
- description
- tags
- capability ID
- workflow type
- functional intent

Search results should clearly show:

- available publicly
- available if friended
- requires manual approval
- requires owner or admin consent

### Relationship Management

Each account needs an inbox and outbox for proposals, counteroffers, approvals, rejections, active grants, consent records, and revocations.

## Failure Handling and Safety

The relay must produce explicit structured failure states:

- target offline
- target unhealthy
- grant revoked
- consent expired
- rate limit exceeded
- policy mismatch
- local target denial

If authorization changes during an active long-running workflow, the job should move into a reviewable `paused` or `cancelled` state rather than continuing invisibly.

Safety enforcement happens twice:

1. The relay validates identity, policy, grant, constraints, quotas, consent, and abuse signals.
2. The target agent validates local safety and domain rules.

Every invocation should produce a durable audit record containing:

- requester account
- source agent
- optional acting member
- target account
- target agent
- capability
- grant ID
- consent record used
- timestamps
- final outcome

## MVP Scope

### Include in v1

- shared account model for individuals and organizations
- members acting through approved source agents
- agent registration, update, suspension, and deletion through MCP
- searchable capability catalogs with full preview
- per-agent access proposals
- counteroffers
- directional grants
- consent records and revocation
- sync relay sessions
- async relay jobs

### Exclude from v1

- billing or paid access
- cross-network federation
- delegated friendship chains
- rich reputation systems
- free-form policy DSL
- direct non-relay transport

## Open Implementation Notes

- Approval routing should support both account-wide admins and agent-specific maintainers.
- The system should treat friendship as mutual identity state and grants as directional operational state.
- Human members should always appear in audit trails when present, even though the effective permission source is still the local agent grant.
