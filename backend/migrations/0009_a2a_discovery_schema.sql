-- Phase D1 of the discovery implementation plan
-- (see docs/specs/2026-04-29-discovery-implementation-plan.md and
--  docs/specs/2026-04-29-discovery-design.md for the full design).
--
-- This migration is *additive*. No existing column is dropped, no
-- existing query path is broken. The new columns are nullable or
-- default-set so old code keeps working unchanged. The CHECK
-- constraint that ties agents.mode to agents.agent_card_url presence
-- lives in 0010 (separate transaction, after any backfill).
--
-- Schema changes correspond exactly to the discovery spec's
-- "Schema deltas" section. Where this migration overrides the
-- earlier migration-doc draft, see the spec's "Migration-doc
-- overrides" section (top of the discovery design doc).

-- ─────────────────────────────────────────────────────────
-- agents — push-vs-pull mode + Agent Card cache + slug lifecycle +
--          tags + heartbeat liveness + mDNS publisher liveness +
--          SEO-noindex flag.
-- ─────────────────────────────────────────────────────────

-- A2A Agent Card storage. agent_card_url is INTENTIONALLY nullable —
-- pull-mode agents have no canonical card URL by construction (they
-- have no public host). The CHECK in 0010 enforces card-or-pull.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS agent_card_url TEXT;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS agent_card_cached JSONB;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS agent_card_fetched_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS agent_card_signed BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS agent_card_signature_verified BOOLEAN NOT NULL DEFAULT false;

-- Push vs pull: 'pull' = no public host, polls the relay's inbox bridge.
-- 'push' = has a public A2A endpoint (canonical URL in agent_card_url).
-- Default 'pull' is correct for everything currently in the table:
-- the scheduler-demo agents are pull-mode by construction, and
-- nothing else has registered yet.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS mode TEXT NOT NULL DEFAULT 'pull'
    CHECK (mode IN ('pull', 'push'));

-- Operator opt-in for mDNS advertisement on the LAN.
-- Default true so a self-hosted server "just works"; can be flipped
-- per-agent for sensitive agents that shouldn't broadcast.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS advertise_via_mdns BOOLEAN NOT NULL DEFAULT true;

-- Free-text tags for v1.5 vertical filtering. Indexed via GIN below.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT '{}';

-- Pull-mode heartbeat: every inbox poll updates this. Server-side
-- now() is authoritative (defends against client clock skew).
ALTER TABLE agents ADD COLUMN IF NOT EXISTS last_polled_at TIMESTAMPTZ;

-- mDNS publisher liveness — set on every successful re-publish so an
-- operator can see "publisher hasn't broadcast in N minutes" in the UI.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS mdns_advertised_at TIMESTAMPTZ;

-- Opt-out for /agents/index.json + crawler exclusion. Default false.
ALTER TABLE agents ADD COLUMN IF NOT EXISTS noindex BOOLEAN NOT NULL DEFAULT false;

-- Slug lifecycle. Tombstoned-on-delete (permanent unless un-tombstoned
-- by the same owner within 365 days). forced_takedown_at marks
-- trademark-dispute outcomes — those slugs become available after a
-- 30-day cooling period (re-registration is then first-come).
ALTER TABLE agents ADD COLUMN IF NOT EXISTS tombstoned_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN IF NOT EXISTS forced_takedown_at TIMESTAMPTZ;

-- ─────────────────────────────────────────────────────────
-- agents — replace full UNIQUE(account_id, slug) with a partial
-- unique index that excludes tombstoned rows.
-- ─────────────────────────────────────────────────────────
-- Today's UNIQUE constraint was created inline by 0003 with the auto-
-- generated name agents_account_id_slug_key. We drop it and replace
-- with a partial unique index so a tombstoned row doesn't block a
-- legitimate re-registration of the same slug by the *same* account
-- after the cooling period.
ALTER TABLE agents DROP CONSTRAINT IF EXISTS agents_account_id_slug_key;
CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_account_slug_live
    ON agents (account_id, slug)
    WHERE tombstoned_at IS NULL;

-- Heartbeat lookups: discovery filters dormant agents by polling time.
CREATE INDEX IF NOT EXISTS idx_agents_last_polled_at
    ON agents (last_polled_at)
    WHERE tombstoned_at IS NULL;

-- Tag containment search (e.g. ?tags=travel,booking).
CREATE INDEX IF NOT EXISTS idx_agents_tags_gin
    ON agents USING GIN (tags);

-- ─────────────────────────────────────────────────────────
-- accounts — slug lifecycle + verified-account badge.
-- ─────────────────────────────────────────────────────────
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS tombstoned_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS forced_takedown_at TIMESTAMPTZ;

-- Verification (DNS-TXT or Google-Workspace SSO). verified_at is when
-- the badge was minted; verification_method records how.
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS verified_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS verification_method TEXT
    CHECK (verification_method IN ('dns_txt', 'google_workspace'));

-- Defensive consistency: a verified_at MUST come with a method.
ALTER TABLE accounts DROP CONSTRAINT IF EXISTS accounts_verified_method_consistency;
ALTER TABLE accounts ADD CONSTRAINT accounts_verified_method_consistency
    CHECK (verified_at IS NULL OR verification_method IS NOT NULL);

-- Replace full UNIQUE(slug) with partial unique on live rows only,
-- same reasoning as agents above.
ALTER TABLE accounts DROP CONSTRAINT IF EXISTS accounts_slug_key;
CREATE UNIQUE INDEX IF NOT EXISTS idx_accounts_slug_live
    ON accounts (slug)
    WHERE tombstoned_at IS NULL;

-- ─────────────────────────────────────────────────────────
-- slug_aliases — rename redirects (90d for agent, 180d for account).
-- ─────────────────────────────────────────────────────────
-- When alice/old-name renames to alice/new-name, we keep the old
-- slug pointing at the new for the configured window so external
-- callers / bookmarks 301-redirect cleanly. After expiry, tombstone.
CREATE TABLE IF NOT EXISTS slug_aliases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 'account' rename → both segments scoped at account; 'agent' rename
    -- → only the agent segment under a fixed account.
    scope TEXT NOT NULL CHECK (scope IN ('account', 'agent')),
    -- Account that owned the slug at rename time. CASCADE on hard delete.
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    old_slug TEXT NOT NULL,
    new_slug TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Rename-window expiry. Before this: 301 to new_slug. After: tombstone.
    expires_at TIMESTAMPTZ NOT NULL,
    -- One alias per (scope, account, old_slug). Repeat renames update
    -- the row in place. The UNIQUE constraint also serves as the
    -- redirect-lookup index — no separate index needed.
    UNIQUE (scope, account_id, old_slug),
    CHECK (expires_at > created_at)
);
-- Cheap secondary index on expiry for the cleanup job that tombstones
-- expired aliases. Postgres rejects partial indexes with non-immutable
-- predicates (now() is STABLE, not IMMUTABLE), so this is a regular
-- btree on expires_at; the cleanup job filters at query time.
CREATE INDEX IF NOT EXISTS idx_slug_aliases_expires_at
    ON slug_aliases (expires_at);

-- ─────────────────────────────────────────────────────────
-- friendships — forward-compat for cross-network import (v2).
-- ─────────────────────────────────────────────────────────
-- Empty for now. A future "import friendship from public network into
-- private network" feature can populate {origin, original_id, ...}.
ALTER TABLE friendships ADD COLUMN IF NOT EXISTS provenance JSONB NOT NULL DEFAULT '{}';

-- ─────────────────────────────────────────────────────────
-- agent_capabilities — A2A skill sync metadata.
-- ─────────────────────────────────────────────────────────
-- The migration doc + discovery spec referred to this table as
-- "capabilities"; the actual table name in our schema (since 0003) is
-- "agent_capabilities". The columns below match the spec exactly.
--
-- synced_from_card: true = capability row was generated by parsing an
-- upstream A2A AgentCard's `skills` list (push agents). false =
-- declared at registration (pull agents). The relay never overwrites
-- ChakraMCP-side policy fields (visibility) regardless of this flag.
ALTER TABLE agent_capabilities ADD COLUMN IF NOT EXISTS synced_from_card BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE agent_capabilities ADD COLUMN IF NOT EXISTS card_skill_id TEXT;
ALTER TABLE agent_capabilities ADD COLUMN IF NOT EXISTS card_skill_name TEXT;
ALTER TABLE agent_capabilities ADD COLUMN IF NOT EXISTS last_synced_at TIMESTAMPTZ;

-- JSONB containment indexes: power capability-shape search ("agents
-- whose output_schema contains {slots: array}"). jsonb_path_ops is
-- smaller and faster than the default for @> queries.
CREATE INDEX IF NOT EXISTS idx_agent_capabilities_output_schema_jsonb
    ON agent_capabilities USING GIN (output_schema jsonb_path_ops);
CREATE INDEX IF NOT EXISTS idx_agent_capabilities_input_schema_jsonb
    ON agent_capabilities USING GIN (input_schema jsonb_path_ops);
