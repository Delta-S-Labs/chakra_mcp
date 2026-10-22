-- Owner consent, PR 1: the tables behind gate 1 (and gate 2 later).
-- Design: docs/superpowers/specs/2026-10-23-owner-consent-design.md §9.
--
-- New tables only, so no lock is taken on anything existing. They have no
-- foreign keys to existing tables either: adding one would lock the busy
-- table it points at (`relay_invocations`, `agents`, `agent_capabilities`,
-- `accounts`, `users`). Deleting a capability or an agent cancels its pending
-- consent requests in code instead, and history is kept.

-- A standing approval for one requesting agent on one capability (§2).
CREATE TABLE consents (
    id                  UUID PRIMARY KEY,
    capability_id       UUID NOT NULL,
    granter_agent_id    UUID NOT NULL,
    grantee_agent_id    UUID NOT NULL,
    purpose             TEXT CHECK (char_length(purpose) <= 500),
    valid_from          TIMESTAMPTZ NOT NULL,
    valid_until         TIMESTAMPTZ NOT NULL,
    max_requests        INT CHECK (max_requests IS NULL OR max_requests > 0),
    used                INT NOT NULL DEFAULT 0
                        CHECK (used >= 0 AND (max_requests IS NULL OR used <= max_requests)),
    approved_by_user_id UUID NOT NULL,
    revoked_at          TIMESTAMPTZ,
    revoked_by_user_id  UUID,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (valid_until > valid_from)
);

-- One unrevoked consent per pair; a new approval revokes the previous one.
CREATE UNIQUE INDEX consents_one_live_per_pair
    ON consents (capability_id, grantee_agent_id)
    WHERE revoked_at IS NULL;

-- One per held request (kind 'request', gate 1) or draft reply (kind
-- 'reply', gate 2).
CREATE TABLE consent_requests (
    id                 UUID PRIMARY KEY,
    kind               TEXT NOT NULL CHECK (kind IN ('request', 'reply')),
    invocation_id      UUID NOT NULL,
    capability_id      UUID NOT NULL,
    granter_agent_id   UUID NOT NULL,
    grantee_agent_id   UUID NOT NULL,
    grantee_account_id UUID NOT NULL,
    status             TEXT NOT NULL
                       CHECK (status IN ('pending', 'approved', 'denied', 'expired', 'cancelled')),
    terms              JSONB NOT NULL,
    approved_terms     JSONB,
    draft              JSONB,
    draft_sha256       TEXT,
    consent_id         UUID REFERENCES consents (id),
    decided_by_user_id UUID,
    decided_at         TIMESTAMPTZ,
    decision_note      TEXT CHECK (char_length(decision_note) <= 500),
    token              TEXT,
    expires_at         TIMESTAMPTZ NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX consent_requests_status_expires ON consent_requests (status, expires_at);
CREATE INDEX consent_requests_granter_status ON consent_requests (granter_agent_id, status);
-- At most one request of each kind per invocation; also how reads find an
-- invocation's consent (relay_invocations gains no column).
CREATE UNIQUE INDEX consent_requests_invocation_kind ON consent_requests (invocation_id, kind);

-- Things an agent is told about out of band of its inbox, e.g. "a request is
-- waiting for your owner's approval". The payload carries ids only; readers
-- join the rest.
CREATE TABLE agent_notices (
    id           UUID PRIMARY KEY,
    agent_id     UUID NOT NULL,
    kind         TEXT NOT NULL,
    payload      JSONB NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ
);

CREATE INDEX agent_notices_undelivered ON agent_notices (agent_id, created_at)
    WHERE delivered_at IS NULL;
CREATE INDEX agent_notices_created ON agent_notices (created_at);
