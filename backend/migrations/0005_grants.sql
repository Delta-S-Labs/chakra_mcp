-- Phase 1.5 — grants: directional capability access between agents.
--
-- A grant says "agent G allows agent X to invoke capability C of agent G".
-- It is the answer to "who can call what" once two agents have shaken
-- hands via an accepted friendship.
--
-- Lifecycle:
--   * active   — currently usable. (Re-)granting after a revoke creates
--                a fresh row; the old row stays as history.
--   * revoked  — cancelled by the granter side. Permanent for that row.
--   * expired  — passed expires_at; flipped lazily by handlers.
--
-- Constraints:
--   * capability_id must belong to granter_agent_id (enforced in app
--     code — Postgres can't easily express that as a column-level CHECK
--     against a foreign table).
--   * granter and grantee must already have an accepted friendship in
--     either direction (enforced in app code at insert time).

CREATE TABLE IF NOT EXISTS grants (
    id UUID PRIMARY KEY,
    granter_agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    grantee_agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    capability_id    UUID NOT NULL REFERENCES agent_capabilities(id) ON DELETE CASCADE,

    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'revoked', 'expired')),

    granted_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ,

    revoked_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    revoked_at    TIMESTAMPTZ,
    revoke_reason TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CHECK (granter_agent_id <> grantee_agent_id)
);

CREATE INDEX IF NOT EXISTS idx_grants_granter ON grants (granter_agent_id);
CREATE INDEX IF NOT EXISTS idx_grants_grantee ON grants (grantee_agent_id);
CREATE INDEX IF NOT EXISTS idx_grants_capability ON grants (capability_id);

-- Only one active grant per (granter, grantee, capability) triple. Revoked
-- and expired rows are kept around as history.
CREATE UNIQUE INDEX IF NOT EXISTS uq_grants_active
    ON grants (granter_agent_id, grantee_agent_id, capability_id)
    WHERE status = 'active';

DROP TRIGGER IF EXISTS grants_updated_at ON grants;
CREATE TRIGGER grants_updated_at BEFORE UPDATE ON grants
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
