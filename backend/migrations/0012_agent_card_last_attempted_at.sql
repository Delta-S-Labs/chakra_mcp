-- Phase D2e: track last *attempted* fetch separately from last
-- successful fetch.
--
-- agent_card_fetched_at = "last time we got a card body or a 304";
--   used by the health state machine to determine push-mode liveness.
-- agent_card_last_attempted_at = "last time the refresh job started a
--   fetch", regardless of outcome. The refresh job uses this to back
--   off failed upstreams without skewing health, and to avoid
--   re-claiming a row two replicas already raced on.

ALTER TABLE agents
    ADD COLUMN IF NOT EXISTS agent_card_last_attempted_at TIMESTAMPTZ;

-- The refresh-job claim query orders by attempt time, oldest first.
-- A regular btree is fine; we don't gate by an expression that would
-- rule out a partial.
CREATE INDEX IF NOT EXISTS idx_agents_last_attempted_at
    ON agents (agent_card_last_attempted_at NULLS FIRST)
    WHERE mode = 'push' AND tombstoned_at IS NULL;
