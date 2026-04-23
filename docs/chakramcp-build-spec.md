# ChakraMCP Network — Build Specification

**This document is the build spec for the ChakraMCP relay network. It is written for Claude Code or any developer implementing the system from scratch in Rust.**

---

## What This Is

ChakraMCP is a managed relay network for MCP-enabled AI agents. Agents register with the network, publish public and friend-gated capabilities, and interact with each other through the relay — never direct peer-to-peer. The network is authoritative for registration, discovery, friendship state, grant state, consent records, audit logs, and session routing. Target agents retain final runtime deny authority.

The system supports individuals and organizations. Each account owns agents. Human members within an account act through approved agents, borrowing that agent's granted permissions when interacting with remote agents.

---

## Technology Stack

| Layer | Choice | Rationale |
|---|---|---|
| Language | **Rust** | Performance, memory safety, strong type system for protocol correctness |
| Web framework | **Axum** | Tokio-native, tower middleware ecosystem, good ergonomics |
| Database | **PostgreSQL** via `sqlx` | Relational model fits the trust/grant/consent schema. Use compile-time checked queries. |
| Async runtime | **Tokio** | Standard for async Rust, required by Axum |
| Serialization | **serde** + **serde_json** | Standard. Use strongly typed request/response structs, not `Value` |
| ID generation | **ulid** or **uuid v7** | Sortable, timestamp-prefixed IDs for events, runs, grants |
| Auth | **JWT** (jsonwebtoken crate) | Bearer tokens for API auth. HMAC-SHA256 for webhook signatures |
| Migration | **sqlx migrate** | Embedded migrations, run on startup |
| Observability | **tracing** + **tracing-subscriber** | Structured logging with span context |
| Config | **Environment variables** via `dotenvy` | 12-factor config, no config files |
| Testing | **cargo test** + **sqlx** test fixtures | Integration tests against real Postgres |
| Deployment | **Docker** → **AWS (ECS Fargate + RDS)** | Containerized on Fargate, managed Postgres on RDS, ALB for ingress |

### Cargo Dependencies (Minimum)

```toml
[dependencies]
axum = { version = "0.8", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "migrate"] }
uuid = { version = "1", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
jsonwebtoken = "9"
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "request-id"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
dotenvy = "0.15"
reqwest = { version = "0.12", features = ["json"] }
thiserror = "2"
anyhow = "1"
```

---

## Project Structure

```
chakramcp-network/
├── Cargo.toml
├── Cargo.lock
├── Dockerfile
├── .env.example
├── migrations/
│   ├── 001_accounts.sql
│   ├── 002_members.sql
│   ├── 003_agents.sql
│   ├── 004_capabilities.sql
│   ├── 005_friendships.sql
│   ├── 006_grants.sql
│   ├── 007_consent.sql
│   ├── 008_events.sql
│   ├── 009_runs.sql
│   └── 010_audit.sql
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── error.rs
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── jwt.rs
│   │   ├── middleware.rs
│   │   └── webhook.rs
│   ├── db/
│   │   ├── mod.rs
│   │   ├── accounts.rs
│   │   ├── agents.rs
│   │   ├── capabilities.rs
│   │   ├── friendships.rs
│   │   ├── grants.rs
│   │   ├── consent.rs
│   │   ├── events.rs
│   │   ├── runs.rs
│   │   └── audit.rs
│   ├── models/
│   │   ├── mod.rs
│   │   ├── account.rs
│   │   ├── member.rs
│   │   ├── agent.rs
│   │   ├── capability.rs
│   │   ├── friendship.rs
│   │   ├── grant.rs
│   │   ├── consent.rs
│   │   ├── event.rs
│   │   ├── run.rs
│   │   └── actor_context.rs
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── agents.rs
│   │   ├── inbox.rs
│   │   ├── events.rs
│   │   ├── runs.rs
│   │   ├── discovery.rs
│   │   ├── proposals.rs
│   │   └── health.rs
│   ├── relay/
│   │   ├── mod.rs
│   │   ├── session.rs
│   │   ├── job.rs
│   │   ├── policy.rs
│   │   └── delivery.rs
│   └── webhook/
│       ├── mod.rs
│       ├── dispatcher.rs
│       └── signer.rs
```

---

## Core Data Model

### Accounts

```sql
CREATE TABLE accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name TEXT NOT NULL,
    account_type TEXT NOT NULL CHECK (account_type IN ('individual', 'organization')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### Members

```sql
CREATE TABLE members (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    email TEXT,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_members_account ON members(account_id);
```

### Agents

```sql
CREATE TABLE agents (
    id TEXT NOT NULL,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'deleted')),
    delivery_polling_enabled BOOLEAN NOT NULL DEFAULT true,
    delivery_webhook_url TEXT,
    webhook_secret_current TEXT,
    webhook_secret_previous TEXT,
    webhook_secret_previous_expires_at TIMESTAMPTZ,
    policy_default_visibility TEXT NOT NULL DEFAULT 'public' CHECK (policy_default_visibility IN ('public', 'friend-gated')),
    tags TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, id)
);
CREATE INDEX idx_agents_status ON agents(status);
CREATE INDEX idx_agents_tags ON agents USING GIN(tags);
```

### Capabilities

```sql
CREATE TABLE capabilities (
    id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tool', 'workflow')),
    description TEXT NOT NULL DEFAULT '',
    visibility TEXT NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'friend-gated')),
    execution_mode TEXT NOT NULL DEFAULT 'sync' CHECK (execution_mode IN ('sync', 'async')),
    consent_mode TEXT CHECK (consent_mode IN ('per-invocation', 'time-boxed', 'persistent-until-revoked')),
    requires_admin BOOLEAN NOT NULL DEFAULT false,
    constraint_schema JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, agent_id, id),
    FOREIGN KEY (account_id, agent_id) REFERENCES agents(account_id, id) ON DELETE CASCADE
);
CREATE INDEX idx_capabilities_visibility ON capabilities(visibility);
CREATE INDEX idx_capabilities_kind ON capabilities(kind);
```

### Friendships

```sql
CREATE TABLE friendships (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_a UUID NOT NULL REFERENCES accounts(id),
    account_b UUID NOT NULL REFERENCES accounts(id),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'dissolved')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_a, account_b),
    CHECK (account_a < account_b)
);
CREATE INDEX idx_friendships_a ON friendships(account_a);
CREATE INDEX idx_friendships_b ON friendships(account_b);
```

### Access Proposals

```sql
CREATE TABLE access_proposals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    requester_account_id UUID NOT NULL REFERENCES accounts(id),
    requester_agent_id TEXT NOT NULL,
    target_account_id UUID NOT NULL REFERENCES accounts(id),
    target_agent_id TEXT NOT NULL,
    requested_capabilities TEXT[] NOT NULL,
    requested_constraints JSONB,
    purpose TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN (
        'pending', 'accepted', 'reduced', 'rejected', 'counteroffered', 'superseded'
    )),
    reviewed_by UUID REFERENCES members(id),
    review_note TEXT,
    counter_capabilities TEXT[],
    counter_constraints JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_proposals_target ON access_proposals(target_account_id, target_agent_id);
CREATE INDEX idx_proposals_requester ON access_proposals(requester_account_id);
CREATE INDEX idx_proposals_status ON access_proposals(status);
```

### Grants

```sql
CREATE TABLE grants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_account_id UUID NOT NULL REFERENCES accounts(id),
    target_agent_id TEXT NOT NULL,
    requester_account_id UUID NOT NULL REFERENCES accounts(id),
    requester_agent_id TEXT NOT NULL,
    capabilities TEXT[] NOT NULL,
    constraints JSONB,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'reduced', 'revoked')),
    expires_at TIMESTAMPTZ,
    rate_limit_per_minute INTEGER,
    acting_member_restrictions UUID[],
    proposal_id UUID REFERENCES access_proposals(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_grants_target ON grants(target_account_id, target_agent_id);
CREATE INDEX idx_grants_requester ON grants(requester_account_id, requester_agent_id);
CREATE INDEX idx_grants_status ON grants(status);
```

### Consent Records

```sql
CREATE TABLE consent_records (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    capability_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('per-invocation', 'time-boxed', 'persistent-until-revoked')),
    approved_by UUID NOT NULL REFERENCES members(id),
    requester_account_id UUID NOT NULL,
    requester_agent_id TEXT NOT NULL,
    grant_id UUID REFERENCES grants(id),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'expired', 'revoked')),
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoke_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_consent_capability ON consent_records(account_id, agent_id, capability_id);
CREATE INDEX idx_consent_status ON consent_records(status);
```

### Events (Inbox)

```sql
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    requester_account_id UUID NOT NULL,
    requester_agent_id TEXT NOT NULL,
    acting_member_id UUID,
    target_account_id UUID NOT NULL,
    target_agent_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    delivery_attempt INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN (
        'pending', 'delivered', 'acknowledged', 'retry_scheduled', 'dead_letter'
    )),
    ack_result TEXT CHECK (ack_result IN ('processed', 'duplicate')),
    ack_handler TEXT,
    ack_at TIMESTAMPTZ,
    nack_reason_code TEXT,
    nack_message TEXT,
    retry_after TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_events_target_status ON events(target_account_id, target_agent_id, status);
CREATE INDEX idx_events_type ON events(event_type);
CREATE INDEX idx_events_retry ON events(status, retry_after) WHERE status = 'retry_scheduled';
```

### Capability Runs

```sql
CREATE TABLE capability_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    capability_id TEXT NOT NULL,
    target_account_id UUID NOT NULL,
    target_agent_id TEXT NOT NULL,
    requester_account_id UUID NOT NULL,
    requester_agent_id TEXT NOT NULL,
    acting_member_id UUID,
    grant_id UUID REFERENCES grants(id),
    consent_record_id UUID REFERENCES consent_records(id),
    callback_mode TEXT NOT NULL CHECK (callback_mode IN ('sync', 'async')),
    input JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'running', 'waiting_for_consent', 'completed', 'failed', 'cancelled'
    )),
    progress INTEGER,
    status_message TEXT,
    output JSONB,
    error JSONB,
    artifacts JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);
CREATE INDEX idx_runs_target ON capability_runs(target_account_id, target_agent_id);
CREATE INDEX idx_runs_status ON capability_runs(status);
CREATE INDEX idx_runs_requester ON capability_runs(requester_account_id, requester_agent_id);
```

### Audit Log

```sql
CREATE TABLE audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type TEXT NOT NULL,
    requester_account_id UUID,
    requester_agent_id TEXT,
    acting_member_id UUID,
    target_account_id UUID,
    target_agent_id TEXT,
    capability_id TEXT,
    grant_id UUID,
    consent_record_id UUID,
    run_id UUID,
    outcome TEXT NOT NULL,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_target ON audit_log(target_account_id, created_at DESC);
CREATE INDEX idx_audit_requester ON audit_log(requester_account_id, created_at DESC);
CREATE INDEX idx_audit_type ON audit_log(event_type, created_at DESC);
```

---

## API Endpoints

All endpoints are versioned under `/v1`. Every endpoint returns JSON. Every error uses the `ErrorEnvelope` shape.

### Agent Registration and Lifecycle

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/agents` | Register a new agent |
| `PATCH` | `/v1/agents/{agent_id}` | Update metadata, delivery, capabilities, policy, tags |
| `GET` | `/v1/agents/{agent_id}` | Fetch current registration |
| `DELETE` | `/v1/agents/{agent_id}` | Deactivate or delete agent |
| `POST` | `/v1/agents/{agent_id}/rotate-secret` | Rotate webhook or API secrets |

### Inbox and Acknowledgements

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/inbox/events` | Poll for pending events (query params: `agent_id`, `limit`, `cursor`) |
| `POST` | `/v1/events/{event_id}/ack` | Acknowledge handled event |
| `POST` | `/v1/events/{event_id}/nack` | Reject event, request retry |

### Run Reporting

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/capability-runs/{run_id}/status` | Report intermediate run status |
| `POST` | `/v1/capability-runs/{run_id}/result` | Submit final result |

### Discovery

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/discovery/agents` | Search agents by name, tags, description, capability |
| `GET` | `/v1/discovery/agents/{agent_id}/capabilities` | List capabilities for a specific agent |

### Access Proposals

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/proposals` | Submit an access proposal |
| `GET` | `/v1/proposals/inbox` | List pending inbound proposals for account |
| `GET` | `/v1/proposals/outbox` | List outbound proposals for account |
| `POST` | `/v1/proposals/{proposal_id}/accept` | Accept a proposal |
| `POST` | `/v1/proposals/{proposal_id}/reject` | Reject a proposal |
| `POST` | `/v1/proposals/{proposal_id}/counteroffer` | Counteroffer with different terms |

### Consent

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/consent/{consent_request_id}/grant` | Approve a consent request |
| `POST` | `/v1/consent/{consent_request_id}/deny` | Deny a consent request |
| `GET` | `/v1/consent/active` | List active consent records for account |
| `POST` | `/v1/consent/{consent_record_id}/revoke` | Revoke a consent record |

### Health

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/healthz` | Basic health check |
| `GET` | `/readyz` | Readiness check (DB connectivity) |

---

## Authentication

### API Auth

Every request includes a bearer token in the `Authorization` header. The token is a JWT containing:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,          // account_id
    pub agent_id: String,   // agent identity
    pub role: String,       // "agent" | "admin" | "member"
    pub exp: i64,           // expiry
    pub iat: i64,           // issued at
}
```

Middleware extracts and validates the token, injects `Claims` into request extensions. All route handlers receive authenticated context.

### Webhook Signing

Outbound webhook deliveries are signed with HMAC-SHA256:

```
signature = HMAC-SHA256(webhook_secret, timestamp + "." + body)
```

Headers sent with every webhook delivery:

```
X-ChakraMCP-Event: {event_type}
X-ChakraMCP-Event-Id: {event_id}
X-ChakraMCP-Timestamp: {unix_timestamp}
X-ChakraMCP-Signature: v1={hex_signature}
```

The agent verifies by recomputing the signature with its stored secret. During rotation, agents should accept signatures from both current and previous secrets.

---

## Event System

### Event Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    FriendshipRequested,
    FriendshipCounteroffered,
    FriendshipAccepted,
    FriendshipRejected,
    GrantUpdated,
    ConsentRequested,
    ConsentGranted,
    ConsentRevoked,
    CapabilityRunRequested,
    CapabilityRunCancelled,
}
```

### Event Envelope

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub event_type: EventType,
    pub occurred_at: DateTime<Utc>,
    pub delivery_attempt: i32,
    pub requester_account_id: Uuid,
    pub requester_agent_id: String,
    pub acting_member_id: Option<Uuid>,
    pub target_account_id: Uuid,
    pub target_agent_id: String,
    pub payload: serde_json::Value,
    pub idempotency_key: String,
}
```

### Delivery

Events are delivered via two channels:

1. **Polling**: Agent calls `GET /v1/inbox/events`. Returns `Vec<EventEnvelope>` + cursor.
2. **Webhooks**: Network pushes `EventEnvelope` to agent's `delivery_webhook_url` with signature headers.

Delivery is at-least-once. Agents must handle duplicates via `event_id` or `idempotency_key`.

### Webhook Dispatcher

Background task that runs continuously:

1. Query for events with status `pending` or `retry_scheduled` where `retry_after <= now()`
2. For each event, check if the target agent has a webhook URL configured
3. POST the EventEnvelope to the webhook URL with signature headers
4. On 2xx: update status to `delivered`
5. On 409: update status to `acknowledged` (duplicate)
6. On 429: schedule retry with `Retry-After` header value if present
7. On 5xx or connection error: increment `delivery_attempt`, schedule retry with exponential backoff (5s, 30s, 2m, 10m, 1h)
8. After 5 failed attempts: move to `dead_letter` status

---

## Relay Engine

### Policy Check Flow

When a capability run is requested, the relay executes this check sequence:

```rust
pub async fn authorize_run(
    ctx: &ActorContext,
    target_agent: &Agent,
    capability_id: &str,
    db: &PgPool,
) -> Result<AuthorizationResult, RelayError> {
    // 1. Verify requester account exists and is active
    // 2. Verify source agent exists and is active
    // 3. Verify target agent exists and is active
    // 4. Check capability visibility (public = skip friendship check for step 5)
    // 5. Check friendship between accounts (required for friend-gated capabilities)
    // 6. Check directional grant exists and is active
    // 7. Check capability is in grant's capability bundle
    // 8. Check grant constraints (expiry, rate limit, member restrictions)
    // 9. Check consent requirement and active consent record
    // 10. Log to audit trail
    // Return: Authorized { grant_id, consent_record_id }
    //       | NeedConsent { consent_request_id }
    //       | Denied { reason }
}
```

### Session (Sync Execution)

For `sync` capabilities:

1. Relay authorizes the call
2. Relay forwards request to target agent's registered endpoint
3. Target agent executes and responds
4. Relay returns result to requester
5. Audit log entry written

### Job (Async Execution)

For `async` capabilities:

1. Relay authorizes the call
2. Relay creates a `capability_run` record with status `queued`
3. Relay delivers a `capability.run.requested` event to target agent
4. Target agent processes, reports status updates via `POST /v1/capability-runs/{run_id}/status`
5. Target agent submits final result via `POST /v1/capability-runs/{run_id}/result`
6. Relay notifies requester of completion

---

## Error Handling

### Error Envelope

```rust
#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
    pub request_id: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
```

### Error Codes

| Status | Code | Meaning |
|---|---|---|
| 400 | `invalid_request` | Malformed body, missing required field |
| 401 | `unauthorized` | Missing or invalid bearer token |
| 403 | `forbidden` | Valid credential, wrong account/agent |
| 404 | `not_found` | Agent, event, or run doesn't exist |
| 409 | `already_processed` | Duplicate event or incompatible state |
| 429 | `rate_limited` | Exceeded throughput |
| 500 | `internal_error` | Unexpected failure |
| 503 | `temporarily_unavailable` | Network or target unavailable |

Use `thiserror` for domain errors, convert to `ErrorEnvelope` in Axum's `IntoResponse` impl.

---

## Implementation Order

Build in this exact sequence. Each phase produces a working, testable system.

### Phase 1: Foundation

1. `main.rs` — Axum server, database pool, config loading, tracing setup
2. `config.rs` — Environment variable parsing
3. `error.rs` — Error types, `IntoResponse` impl for `ErrorEnvelope`
4. Database migrations (all tables)
5. `GET /healthz` and `GET /readyz`

**Checkpoint: server boots, connects to Postgres, responds to health checks.**

### Phase 2: Registration

1. `POST /v1/agents` — create agent record + capabilities
2. `GET /v1/agents/{agent_id}` — fetch agent
3. `PATCH /v1/agents/{agent_id}` — partial update
4. `DELETE /v1/agents/{agent_id}` — soft delete (set status = deleted)
5. `POST /v1/agents/{agent_id}/rotate-secret` — secret rotation with overlap
6. JWT auth middleware

**Checkpoint: agents can register, update, and authenticate.**

### Phase 3: Discovery

1. `GET /v1/discovery/agents` — full-text search on name, description, tags
2. `GET /v1/discovery/agents/{agent_id}/capabilities` — list capabilities with visibility filtering

**Checkpoint: agents are discoverable.**

### Phase 4: Proposals and Friendships

1. `POST /v1/proposals` — submit access proposal
2. `GET /v1/proposals/inbox` and `/outbox` — list proposals
3. `POST /v1/proposals/{id}/accept` — accept, create friendship + grant
4. `POST /v1/proposals/{id}/reject` — reject
5. `POST /v1/proposals/{id}/counteroffer` — counteroffer with different terms
6. Friendship creation on first accepted proposal
7. Grant creation from accepted/reduced proposal

**Checkpoint: accounts can form relationships and negotiate access.**

### Phase 5: Event System and Inbox

1. Event creation helper (insert into events table)
2. `GET /v1/inbox/events` — polling endpoint with cursor pagination
3. `POST /v1/events/{event_id}/ack` — acknowledge
4. `POST /v1/events/{event_id}/nack` — reject with retry
5. Events emitted on: proposal submitted, accepted, rejected, counteroffered, grant updated

**Checkpoint: agents receive typed events through polling.**

### Phase 6: Consent

1. `POST /v1/consent/{id}/grant` — approve consent request
2. `POST /v1/consent/{id}/deny` — deny
3. `GET /v1/consent/active` — list active records
4. `POST /v1/consent/{id}/revoke` — revoke
5. Consent check integrated into relay policy

**Checkpoint: sensitive capabilities require explicit approval.**

### Phase 7: Relay and Execution

1. Policy check function (the 10-step authorization)
2. Capability run creation for async workflows
3. `POST /v1/capability-runs/{run_id}/status` — status updates
4. `POST /v1/capability-runs/{run_id}/result` — final result
5. Event emission for `capability.run.requested` and `capability.run.cancelled`
6. Audit log writes on every invocation

**Checkpoint: agents can execute remote capabilities through the relay.**

### Phase 8: Webhook Delivery

1. Webhook signature generation
2. Background dispatcher task (Tokio spawn)
3. Retry logic with exponential backoff
4. Dead letter handling after max attempts
5. Health probe forwarding for webhook-enabled agents

**Checkpoint: events can be pushed to agents instead of only polled.**

### Phase 9: Hardening

1. Rate limiting middleware (tower layer)
2. Request ID propagation
3. Input validation (reject oversized payloads, malformed UUIDs)
4. Pagination limits
5. Graceful shutdown
6. Integration test suite

**Checkpoint: production-ready.**

---

## Deployment

### Dockerfile

```dockerfile
FROM rust:1.87 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/chakramcp-network /usr/local/bin/
COPY --from=builder /app/migrations /app/migrations
ENV RUST_LOG=info
EXPOSE 8080
CMD ["chakramcp-network"]
```

### Environment Variables

```bash
DATABASE_URL=postgres://user:pass@host:5432/chakramcp
JWT_SECRET=<random-256-bit-key>
WEBHOOK_SIGNING_SECRET=<random-256-bit-key>
PORT=8080
RUST_LOG=info,chakramcp_network=debug
```

### AWS Deployment

**Infrastructure (managed via CloudFormation, CDK, or Terraform):**

**Compute — ECS Fargate:**
- Single ECS service running the Docker container
- Fargate launch type (no EC2 instances to manage)
- Minimum 1 task, auto-scale to 4 based on CPU/memory
- Deploy via ECR (push image to Elastic Container Registry, ECS pulls from there)
- Task definition injects environment variables from AWS Secrets Manager and SSM Parameter Store

**Database — RDS PostgreSQL:**
- RDS PostgreSQL 16, `db.t4g.medium` for launch (2 vCPU, 4GB RAM, burstable)
- Multi-AZ disabled initially (enable when traffic justifies cost)
- Automated backups enabled, 7-day retention
- Security group allows inbound only from ECS Fargate security group
- Connection via `DATABASE_URL` stored in Secrets Manager

**Networking:**
- VPC with public and private subnets across 2 AZs
- ALB (Application Load Balancer) in public subnets, handles TLS termination
- ECS tasks run in private subnets, accessible only through ALB
- NAT Gateway in one AZ for outbound traffic (webhook delivery, external API calls)
- Route 53 for DNS (`api.chakramcp.com` → ALB)
- ACM certificate for TLS (auto-renewed)

**Secrets and Config:**
- `JWT_SECRET` → Secrets Manager
- `WEBHOOK_SIGNING_SECRET` → Secrets Manager
- `DATABASE_URL` → Secrets Manager (auto-constructed from RDS endpoint)
- `PORT`, `RUST_LOG` → SSM Parameter Store or task definition environment
- ECS task role has read access to the specific secrets and parameters

**Observability:**
- Container logs → CloudWatch Logs (structured JSON via `tracing-subscriber`)
- ALB access logs → S3
- RDS performance insights enabled
- CloudWatch alarms: ECS task health, RDS CPU, RDS connections, ALB 5xx rate

**CI/CD Pipeline:**
- GitHub Actions or CodePipeline
- On push to `main`: build Docker image → push to ECR → update ECS service (rolling deploy)
- ECS performs health check against `/healthz` before routing traffic to new tasks

**Estimated Monthly Cost (Launch):**

| Service | Cost |
|---|---|
| ECS Fargate (1 task, 0.5 vCPU, 1GB) | ~$15 |
| RDS PostgreSQL (db.t4g.medium, single-AZ) | ~$55 |
| ALB | ~$22 |
| NAT Gateway | ~$35 |
| Route 53 + ACM | ~$1 |
| CloudWatch Logs | ~$5 |
| ECR storage | ~$1 |
| **Total** | **~$134/month** |

**Cost Notes:**
- NAT Gateway is the biggest surprise cost. If webhook delivery volume is low initially, consider removing NAT and routing outbound through the ALB or using a VPC endpoint for ECR/Secrets Manager.
- RDS can start at `db.t4g.micro` (~$15/month) if traffic is minimal, but `t4g.medium` avoids connection limit issues under moderate load.
- Fargate pricing scales linearly with tasks. At 4 tasks under load, compute is ~$60/month.

**Migration from dev to production:**
- Dev: single Fargate task, single-AZ RDS, no NAT (use public subnet for dev tasks)
- Production: auto-scaling Fargate, multi-AZ RDS, NAT Gateway, CloudWatch alarms
- The Docker image and application code are identical. Only infrastructure config changes.

The server runs migrations on startup via `sqlx::migrate!()`. On first deploy, the ECS task boots, connects to RDS, runs all pending migrations, and begins serving traffic.

---

## Testing Strategy

### Unit Tests

- JWT encoding/decoding
- Webhook signature generation and verification
- Policy check logic (all 10 steps, each with pass/fail cases)
- Event type serialization/deserialization

### Integration Tests

- Full registration → discovery → proposal → grant → execution flow
- Consent flow: request → approve → use → revoke → deny
- Polling: create events → poll → ack → verify cleared
- Webhook delivery: mock target endpoint, verify signatures and retries
- Error cases: expired grants, revoked consent, rate limits, duplicate events

---

## What This System Does NOT Include (Out of Scope for v1)

- User-facing web UI (separate frontend project)
- Token economy / ad system (separate service)
- Managed agent runtime (separate service)
- Creator marketplace (separate service)
- Billing or payments
- Cross-network federation
- Reputation scoring
- Direct peer transport
- MCP transport layer (agents connect via HTTP REST in v1, MCP-native transport is a future layer)
