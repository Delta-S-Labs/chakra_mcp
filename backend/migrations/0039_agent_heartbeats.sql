-- When each agent last pulled its inbox, for the directory's "online" dot
-- and for hiding dormant agents (house agents spec,
-- docs/superpowers/specs/2026-10-15-house-agents-design.md §3).
--
-- Not `agents.last_polled_at` (migration 0009, never written): `agents` has a
-- BEFORE UPDATE trigger that bumps `updated_at`, so a heartbeat there would
-- turn "last edited" into "last polled", and it's a busy, indexed table.
-- The relay records polls in memory and upserts them here in one batch
-- every 30 s, so a poll never waits on a write.
--
-- No foreign key: creating one would lock `agents`, and a row for a deleted
-- agent is harmless (readers join through `agents`).
CREATE TABLE agent_heartbeats (
    agent_id       UUID        PRIMARY KEY,
    last_polled_at TIMESTAMPTZ NOT NULL
);
