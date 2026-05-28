-- D-public-invoke/1: per-capability "public-invokable" flag + owner-set
-- per-invoker monthly quota.
--
-- Today an agent's `visibility` controls *discoverability*; invocation is
-- always gated by friendship → grant → invoke. This adds a third gate at
-- the capability level: an owner may opt a network-visible capability into
-- being callable by any *registered* agent without a friendship/grant.
-- The relay handler reads `public_invoke` to short-circuit the grant path
-- and `public_monthly_quota_per_agent` to enforce a per-invoker monthly
-- cap (counted against relay_invocations).
--
-- Additive + safe:
--   • new columns default to off / null
--   • two CHECKs only bind the new public path
--   • no backfill — existing capabilities remain friend-only
--   • feature is dark until an owner explicitly opts in
--
-- Sub-project 1 of the agent ratings/reviews feature; see
-- docs/superpowers/specs/2026-05-23-public-invokable-capabilities-design.md.

ALTER TABLE agent_capabilities
    ADD COLUMN IF NOT EXISTS public_invoke BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS public_monthly_quota_per_agent INTEGER;

-- public-invokable implies network-discoverable: strangers can't be allowed
-- to call something they can't even see in the directory.
ALTER TABLE agent_capabilities
    DROP CONSTRAINT IF EXISTS cap_public_requires_network;
ALTER TABLE agent_capabilities
    ADD CONSTRAINT cap_public_requires_network
    CHECK (NOT public_invoke OR visibility = 'network');

-- A public capability must carry a quota — NULL while public_invoke=true is
-- nonsensical (it would mean "publicly invokable with no abuse control").
-- The application layer additionally enforces quota >= 1.
ALTER TABLE agent_capabilities
    DROP CONSTRAINT IF EXISTS cap_public_requires_quota;
ALTER TABLE agent_capabilities
    ADD CONSTRAINT cap_public_requires_quota
    CHECK (NOT public_invoke OR public_monthly_quota_per_agent IS NOT NULL);

-- Composite index that supports the per-invoker monthly quota count
-- (COUNT(*) WHERE grantee_agent_id = $1 AND capability_id = $2 AND
-- created_at >= date_trunc('month', now())). The existing
-- idx_invocations_grantee covers (grantee_agent_id, created_at); adding
-- capability_id makes the quota check an index-only scan.
CREATE INDEX IF NOT EXISTS idx_invocations_grantee_capability_created
    ON relay_invocations (grantee_agent_id, capability_id, created_at DESC);
