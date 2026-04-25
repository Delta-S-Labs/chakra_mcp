-- Phase 1.5 — agents and their capabilities.
--
-- An agent belongs to one account (personal or organization). The
-- account's members are the humans who can edit it. Visibility controls
-- discovery on the network: 'private' = only the owning account sees it,
-- 'network' = listed in /v1/network/agents for everyone on this network.
--
-- Capabilities are the named operations an agent exposes. Each has a
-- JSON Schema describing input/output. Per-capability visibility lets an
-- agent be discoverable while keeping a subset of its capabilities
-- internal.
--
-- Friendships, grants, and audit log live in later migrations.

-- ─────────────────────────────────────────────────────────
-- agents
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS agents (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,

    -- url-safe handle, unique within an account so links stay stable
    -- even if display name changes.
    slug TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',

    -- 'private' (default) | 'network'
    visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'network')),

    -- Where the relay reaches this agent. Null until the operator wires
    -- a webhook target; agents without an endpoint are still visible
    -- but cannot be invoked.
    endpoint_url TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (account_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_agents_account ON agents (account_id);
CREATE INDEX IF NOT EXISTS idx_agents_visibility ON agents (visibility) WHERE visibility = 'network';

DROP TRIGGER IF EXISTS agents_updated_at ON agents;
CREATE TRIGGER agents_updated_at BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ─────────────────────────────────────────────────────────
-- capabilities — named operations an agent exposes
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS agent_capabilities (
    id UUID PRIMARY KEY,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,

    -- snake_case identifier, unique within an agent.
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',

    -- JSON Schema for input/output. We don't validate here at the SQL
    -- layer — the relay validates against these at invoke time.
    input_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    output_schema JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- 'private' | 'network'. A network-visible agent can still hide
    -- specific capabilities; private capabilities only run for users
    -- inside the owning account.
    visibility TEXT NOT NULL DEFAULT 'network'
        CHECK (visibility IN ('private', 'network')),

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (agent_id, name)
);

CREATE INDEX IF NOT EXISTS idx_agent_capabilities_agent ON agent_capabilities (agent_id);

DROP TRIGGER IF EXISTS agent_capabilities_updated_at ON agent_capabilities;
CREATE TRIGGER agent_capabilities_updated_at BEFORE UPDATE ON agent_capabilities
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
