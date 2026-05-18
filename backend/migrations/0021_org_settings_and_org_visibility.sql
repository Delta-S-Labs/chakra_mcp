-- D-orgs/1: per-org settings + 'org' as a third agent visibility tier.
--
-- Two product changes land together because the visibility tier and the
-- settings columns are useless without each other:
--
--   1. accounts.default_agent_visibility (TEXT, default 'private')
--      The pre-filled visibility for agents created under this account.
--      Doesn't enforce; users can still override at create-time. The
--      enforcement comes from the visibility CHECK on agents (below) +
--      whatever the UI surfaces.
--
--   2. accounts.auto_friendship_enabled (BOOLEAN, default false)
--      When true, agents owned by accounts that share membership in
--      this org get instantly-accepted friendships with each other.
--      This migration only adds the column; the auto-friendship backfill
--      and triggers ship in a follow-up so failure to enforce can't
--      brick a deploy. Reading "false" means "no behaviour change yet".
--
--   3. agents.visibility / agent_capabilities.visibility now accept 'org'
--      in addition to 'private' and 'network'. Discovery's tsquery /
--      `/v1/discovery/agents` continues to filter `visibility = 'network'`
--      so 'org' agents stay out of the public surface. The authed
--      `/v1/network/agents` will be widened in the follow-up to include
--      'org' agents that the caller can actually see via membership.
--
-- All defaults match current behaviour, so this migration is a no-op for
-- existing rows. Forward + back safe (no DROP).

ALTER TABLE accounts
    ADD COLUMN IF NOT EXISTS default_agent_visibility TEXT NOT NULL DEFAULT 'private',
    ADD COLUMN IF NOT EXISTS auto_friendship_enabled BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE accounts
    DROP CONSTRAINT IF EXISTS accounts_default_agent_visibility_check;
ALTER TABLE accounts
    ADD CONSTRAINT accounts_default_agent_visibility_check
    CHECK (default_agent_visibility IN ('private', 'org', 'network'));

-- Widen the visibility CHECK on agents to admit 'org'. CHECK constraints
-- in Postgres are anchored to a name we can't predict (Postgres auto-names
-- them); the safest portable approach is: ADD CONSTRAINT with a known
-- name and rely on the new CHECK being a strict superset of the old one.
-- The old anonymous CHECK still passes for 'private' and 'network', so
-- both constraints can co-exist until we drop the old one in a future
-- cleanup migration. (The named version below is what gets enforced
-- going forward; the old anonymous one is now redundant.)
ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS agents_visibility_check;
ALTER TABLE agents
    ADD CONSTRAINT agents_visibility_check
    CHECK (visibility IN ('private', 'org', 'network'));

ALTER TABLE agent_capabilities
    DROP CONSTRAINT IF EXISTS agent_capabilities_visibility_check;
ALTER TABLE agent_capabilities
    ADD CONSTRAINT agent_capabilities_visibility_check
    CHECK (visibility IN ('private', 'org', 'network'));
