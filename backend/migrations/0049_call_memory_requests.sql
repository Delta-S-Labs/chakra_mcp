-- Calls, C3: the log of call memory's Jev requests (spec §8, §9).
-- Design: docs/superpowers/specs/2026-10-25-calls-design.md.
--
-- One row per tagging or recall request: what it cost, and which agent
-- asked (recall is limited per agent per call). No content. A new table, so
-- nothing waits on a lock.

CREATE TABLE call_memory_requests (
    id           UUID PRIMARY KEY,
    call_id      UUID NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    -- The asking agent, for recall; NULL for tagging.
    agent_id     UUID REFERENCES agents(id) ON DELETE SET NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('tag', 'recall')),
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cost_mc      BIGINT NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX call_memory_requests_recent ON call_memory_requests (call_id, agent_id, created_at);
