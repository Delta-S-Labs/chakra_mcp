-- D10a: discovery search infrastructure.
--
-- Adds the columns + indexes the new `/v1/discovery/agents`
-- handler needs for fast filter-combined search at v1 scale (10k
-- agents). Per discovery design §"D10a Discovery search":
--
-- - search_vec: tsvector aggregating display_name, description,
--   account display_name, capability descriptions/names, and tags.
--   GIN index for `q` filter. Trigger keeps it fresh on
--   row mutations to agents AND agent_capabilities.
-- - friend_count: counter cache for trending. Updated by a tokio
--   background job (D10a-2) reading the friendships table; D10a's
--   migration only adds the column with a default 0.
-- - recent_invocations_7d: same idea, derived from relay_invocations
--   in a sliding 7-day window. Background-refreshed.
--
-- Capability-shape search uses idx_agent_capabilities_output_schema_jsonb
-- (already added in 0009).

ALTER TABLE agents ADD COLUMN IF NOT EXISTS friend_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS recent_invocations_7d INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS search_vec tsvector;

-- Recompute search_vec for one row from its current fields + the
-- agent's owning account display name + the tag array. Capability
-- text is added in a separate trigger so an agent's tsvector
-- updates when its capabilities change.
CREATE OR REPLACE FUNCTION agents_search_vec_compose(
    display_name TEXT,
    description  TEXT,
    account_display_name TEXT,
    tags TEXT[]
) RETURNS tsvector AS $$
    SELECT
        setweight(to_tsvector('simple', coalesce(display_name, '')), 'A')
        || setweight(to_tsvector('simple', coalesce(account_display_name, '')), 'B')
        || setweight(to_tsvector('simple', coalesce(description, '')), 'C')
        || setweight(to_tsvector('simple', array_to_string(coalesce(tags, '{}'::text[]), ' ')), 'B')
$$ LANGUAGE SQL IMMUTABLE;

-- Backfill: compute search_vec for existing rows. The capability
-- text contribution is appended below.
UPDATE agents a
   SET search_vec = agents_search_vec_compose(
        a.display_name,
        a.description,
        (SELECT display_name FROM accounts WHERE id = a.account_id),
        a.tags
   );

-- Trigger to keep search_vec current when an agent row mutates.
CREATE OR REPLACE FUNCTION agents_refresh_search_vec() RETURNS trigger AS $$
DECLARE
    acct_name TEXT;
    cap_text  TEXT;
BEGIN
    SELECT display_name INTO acct_name FROM accounts WHERE id = NEW.account_id;
    SELECT coalesce(
        string_agg(coalesce(name, '') || ' ' || coalesce(description, ''), ' '),
        ''
    )
      INTO cap_text
      FROM agent_capabilities WHERE agent_id = NEW.id;
    NEW.search_vec :=
        agents_search_vec_compose(NEW.display_name, NEW.description, acct_name, NEW.tags)
        || setweight(to_tsvector('simple', cap_text), 'C');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS agents_search_vec_update ON agents;
CREATE TRIGGER agents_search_vec_update
    BEFORE INSERT OR UPDATE OF display_name, description, account_id, tags ON agents
    FOR EACH ROW EXECUTE FUNCTION agents_refresh_search_vec();

-- When a capability changes, recompute its agent's search_vec by
-- triggering an UPDATE that touches updated_at (the trigger above
-- runs because account_id is in its column list, but a no-op
-- UPDATE on the agent row still fires UPDATE triggers).
CREATE OR REPLACE FUNCTION agent_capabilities_refresh_owner_vec() RETURNS trigger AS $$
DECLARE
    target_agent UUID;
BEGIN
    target_agent := COALESCE(NEW.agent_id, OLD.agent_id);
    UPDATE agents SET updated_at = now() WHERE id = target_agent;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS agent_capabilities_refresh_search_vec ON agent_capabilities;
CREATE TRIGGER agent_capabilities_refresh_search_vec
    AFTER INSERT OR UPDATE OR DELETE ON agent_capabilities
    FOR EACH ROW EXECUTE FUNCTION agent_capabilities_refresh_owner_vec();

-- Also fire the UPDATE trigger on agents when its updated_at column
-- moves so the trigger above executes. The original trigger only
-- watched display_name/description/account_id/tags; widen to
-- updated_at so the capability cascade works.
DROP TRIGGER IF EXISTS agents_search_vec_update ON agents;
CREATE TRIGGER agents_search_vec_update
    BEFORE INSERT OR UPDATE OF
        display_name, description, account_id, tags, updated_at
    ON agents
    FOR EACH ROW EXECUTE FUNCTION agents_refresh_search_vec();

-- One more backfill so the capability text contribution is included.
UPDATE agents SET updated_at = updated_at;  -- no-op UPDATE re-runs the trigger

CREATE INDEX IF NOT EXISTS idx_agents_search_vec ON agents USING GIN (search_vec);

-- Composite index for the dominant filter combination
-- (network-visibility, non-tombstoned, ordered by recency).
CREATE INDEX IF NOT EXISTS idx_agents_discovery_default
    ON agents (created_at DESC)
    WHERE visibility = 'network' AND tombstoned_at IS NULL;
