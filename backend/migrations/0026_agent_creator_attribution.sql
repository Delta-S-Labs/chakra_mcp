-- Agent creator attribution: record WHICH credential issuer created an
-- agent, not just which human (agents.created_by_user_id already covers
-- the human, since migration 0003).
--
-- This is the lineage the upcoming scoped-agent-grants work needs for an
-- "own" scope — "the set of agents this OAuth client (or API key) created,
-- across all of its tokens". Attribution is to the stable issuer (the
-- oauth_clients.client_id / api_keys.id), NOT to an ephemeral JWT, so the
-- managed set survives token rotation: a fresh token for the same app can
-- still recognise the agents an older token created.
--
-- Both columns are nullable. NULL means "created outside a client/key
-- context" — i.e. the web dashboard or a pre-0026 row. Nothing reads these
-- for authorization yet (that lands with credential_scopes); this migration
-- is pure data capture and changes no behaviour.
--
-- ON DELETE SET NULL mirrors agents.created_by_user_id: deleting a client
-- or key drops the attribution (the agent stays, just unattributed) rather
-- than cascading the agent away.

ALTER TABLE agents
    ADD COLUMN IF NOT EXISTS created_by_client_id  TEXT
        REFERENCES oauth_clients(client_id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS created_by_api_key_id UUID
        REFERENCES api_keys(id) ON DELETE SET NULL;

-- Partial indexes keep the "agents this client/key created" lookup cheap
-- once the 'own' scope check starts filtering on these.
CREATE INDEX IF NOT EXISTS idx_agents_created_by_client
    ON agents (created_by_client_id)
    WHERE created_by_client_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_agents_created_by_api_key
    ON agents (created_by_api_key_id)
    WHERE created_by_api_key_id IS NOT NULL;
