-- Phase 1.5 — friendships between agents.
--
-- A friendship is an agent-to-agent social tie. It says "we know each
-- other and accept relay traffic between us." It does NOT grant the
-- right to invoke specific capabilities — that's `grants` in 0005.
--
-- Lifecycle: proposed → accepted | rejected | cancelled | countered.
--   * accepted is the only state in which subsequent grants are allowed
--     (enforced at the application layer).
--   * countered means the recipient rejected the original AND opened a
--     fresh proposal in the reverse direction. The new proposal points
--     back via `counter_of_id` so the UI can thread the conversation.
--   * cancelled is the proposer's withdrawal before a decision.
--
-- An accepted friendship can later be ended; that's a NEW row of
-- status='cancelled' from either party (i.e. cancellation is also the
-- "unfriend" verb). Or we add an explicit 'ended' status later — we
-- don't need it for milestone B.

CREATE TABLE IF NOT EXISTS friendships (
    id UUID PRIMARY KEY,
    proposer_agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    target_agent_id   UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,

    status TEXT NOT NULL DEFAULT 'proposed'
        CHECK (status IN ('proposed', 'accepted', 'rejected', 'cancelled', 'countered')),

    proposer_message TEXT,
    response_message TEXT,

    -- When this row is the counter to an earlier proposal, this points
    -- to the original (now status='countered') friendship row.
    counter_of_id UUID REFERENCES friendships(id) ON DELETE SET NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at TIMESTAMPTZ,

    CHECK (proposer_agent_id <> target_agent_id)
);

CREATE INDEX IF NOT EXISTS idx_friendships_proposer ON friendships (proposer_agent_id);
CREATE INDEX IF NOT EXISTS idx_friendships_target ON friendships (target_agent_id);
CREATE INDEX IF NOT EXISTS idx_friendships_counter_of ON friendships (counter_of_id);

-- Only one live proposal-or-acceptance per ordered (proposer, target)
-- pair. Re-proposing after a rejection is fine; this only blocks
-- duplicates while a relationship is in flight.
CREATE UNIQUE INDEX IF NOT EXISTS uq_friendships_active_pair
    ON friendships (proposer_agent_id, target_agent_id)
    WHERE status IN ('proposed', 'accepted');

DROP TRIGGER IF EXISTS friendships_updated_at ON friendships;
CREATE TRIGGER friendships_updated_at BEFORE UPDATE ON friendships
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
