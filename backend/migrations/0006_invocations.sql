-- Phase 1.5 — relay invocations + audit log.
--
-- One row per attempted invocation through POST /v1/invoke. We persist
-- the row regardless of outcome (rejected pre-flight, succeeded on
-- the wire, failed at the agent, timed out, etc.) so the audit log is
-- a complete record of every attempted call.
--
-- We snapshot capability_name at write time because capabilities can
-- be renamed; the audit log should still read correctly years later
-- even if the capability row is later edited or deleted.

CREATE TABLE IF NOT EXISTS relay_invocations (
    id UUID PRIMARY KEY,

    -- All four FKs are nullable because callers can include identifiers
    -- that get rejected during validation, and we want to keep the row
    -- even if a referenced row is later deleted.
    grant_id          UUID REFERENCES grants(id)              ON DELETE SET NULL,
    granter_agent_id  UUID REFERENCES agents(id)              ON DELETE SET NULL,
    grantee_agent_id  UUID REFERENCES agents(id)              ON DELETE SET NULL,
    capability_id     UUID REFERENCES agent_capabilities(id)  ON DELETE SET NULL,
    capability_name   TEXT NOT NULL,

    invoked_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,

    -- 'rejected' — failed pre-flight (no grant, expired grant, missing
    --              endpoint, etc.) before any webhook attempt.
    -- 'succeeded' — webhook returned 2xx with valid JSON.
    -- 'failed'    — webhook returned non-2xx, or returned a body the
    --              relay couldn't parse.
    -- 'timeout'   — webhook didn't respond within the relay's deadline.
    status TEXT NOT NULL CHECK (status IN ('rejected', 'succeeded', 'failed', 'timeout')),

    http_status   INTEGER,
    elapsed_ms    INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,

    -- Truncated for storage. The relay caps each side at 16KB; anything
    -- larger gets a marker and a length annotation.
    input_preview  JSONB,
    output_preview JSONB,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_invocations_granter ON relay_invocations (granter_agent_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_invocations_grantee ON relay_invocations (grantee_agent_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_invocations_grant   ON relay_invocations (grant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_invocations_invoker ON relay_invocations (invoked_by_user_id, created_at DESC);
