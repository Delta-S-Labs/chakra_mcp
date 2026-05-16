-- Attribute platform actions to the user who performed them, so the
-- /app/usage "Personal" scope on the upcoming by-platform-action
-- breakdown can distinguish a caller's own activity from their
-- teammates'. Pre-existing rows stay NULL — same shape as the
-- api_key_id (migration 0016, #54) and minted_jti (migration 0018,
-- #64) attribution columns. Personal-scope queries treat NULL as
-- "not attributable" and exclude. Org-scope queries ignore the
-- column entirely and stay membership-scoped.
--
-- # Scope of this migration
--
-- Only adds columns that are NOT already in the schema. During
-- implementation we found three of the six columns the design spec
-- originally listed are already present from earlier migrations
-- and already wired by handlers (so attribution was actually fine
-- for them all along — the gap was just on the /usage query side):
--
--   * agents.created_by_user_id           — migration 0003 (used by
--                                            relay/handlers/agents.rs:357
--                                            and app/handlers/oauth.rs:958)
--   * grants.granted_by_user_id           — migration 0005 (used by
--                                            relay/handlers/grants.rs:323)
--   * grants.revoked_by_user_id           — migration 0005 (used by
--                                            relay/handlers/grants.rs:386)
--
-- So this migration adds only the genuinely-missing three:
--
--   * friendships.proposer_user_id        — set by relay/handlers/
--                                            friendships.rs::propose,
--                                            ::counter, mcp.rs::handle
--   * friendships.decided_by_user_id      — set by relay/handlers/
--                                            friendships.rs::{accept,
--                                            reject, cancel}
--   * agent_capabilities.created_by_user_id
--                                          — set by relay/handlers/
--                                            capabilities.rs::create
--
-- All three are nullable. Test fixtures and shadow-creation paths
-- inside #[cfg(test)] mods continue to leave them NULL — no test
-- changes required for this migration or the binding-site PR.
--
-- See docs/specs/2026-05-16-usage-action-bifurcation-design.md for
-- the full design (revised with the existing-column finding).

ALTER TABLE friendships
    ADD COLUMN IF NOT EXISTS proposer_user_id   UUID REFERENCES users(id);
ALTER TABLE friendships
    ADD COLUMN IF NOT EXISTS decided_by_user_id UUID REFERENCES users(id);

ALTER TABLE agent_capabilities
    ADD COLUMN IF NOT EXISTS created_by_user_id UUID REFERENCES users(id);
