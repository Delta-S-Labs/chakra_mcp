-- MCP and discovery D3: default agents (spec §6.4).
-- Design: docs/superpowers/specs/2026-10-27-mcp-discovery-design.md.
--
-- The agent the relay makes for a caller who calls an agent by name over
-- MCP with no agent of their own to call as: a personal agent for an
-- unscoped credential (key `user:<user_id>`), or a client agent for an
-- `own`-scoped one with none (`oauth:<client_id>:<user_id>` or
-- `api_key:<id>`). A deleted default agent is replaced on next use, which
-- updates the row.
--
-- A new table nothing reads yet: no lock on a hot table.

CREATE TABLE IF NOT EXISTS default_agents (
    key        TEXT PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    agent_id   UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The agents list labels default agents.
CREATE INDEX IF NOT EXISTS idx_default_agents_agent ON default_agents (agent_id);
