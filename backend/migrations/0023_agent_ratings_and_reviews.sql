-- D-ratings/1: agent-to-agent ratings & reviews.
--
-- One review per (reviewer_agent, target_agent), editable in place
-- (no hard delete; owner can soft-hide). Each review carries 1-5 stars,
-- an optional comment, and a `tier` ('friend' | 'public') stamped at
-- write-time from the relationship state then. ≥1 tagged capability
-- the reviewer has invoked (`relay_invocations.status != 'rejected'`)
-- is required by the application layer; the agent_review_tags join
-- table stores those tags.
--
-- Sub-project 2 of the ratings feature; builds on the public-invoke
-- tier from migration 0022. See
-- docs/superpowers/specs/2026-05-28-agent-ratings-and-reviews-design.md.
--
-- Additive + safe: two new tables, no backfill, no touch of existing
-- rows. Feature is dark until reviews accumulate; aggregate fields on
-- the broader DTOs default to NULL/0 on agents that haven't been
-- reviewed.

CREATE TABLE IF NOT EXISTS agent_reviews (
    id                  UUID PRIMARY KEY,
    reviewer_agent_id   UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    target_agent_id     UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    rating              SMALLINT NOT NULL,
    comment             TEXT,
    -- Tier stamped at write-time. 'friend' = accepted friendship between
    -- reviewer + target existed; 'public' = no friendship, usage proven
    -- via a public_invoke=true capability call (migration 0022).
    tier                TEXT NOT NULL,
    -- Soft-hide: target's owner can hide a review they consider abusive.
    -- Hidden reviews are excluded from aggregates + the public list but
    -- the row stays for audit. Owner can also un-hide.
    hidden_at           TIMESTAMPTZ,
    hidden_by_user_id   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT reviews_no_self_review CHECK (reviewer_agent_id <> target_agent_id),
    CONSTRAINT reviews_rating_bounds  CHECK (rating BETWEEN 1 AND 5),
    CONSTRAINT reviews_tier_known     CHECK (tier IN ('friend', 'public')),
    CONSTRAINT reviews_one_per_pair   UNIQUE (reviewer_agent_id, target_agent_id)
);

-- Dominant access pattern: "list / count / average the live (un-hidden)
-- reviews of a target agent, newest first." This partial index covers
-- the directory aggregate subqueries AND the list endpoint's pagination.
CREATE INDEX IF NOT EXISTS idx_reviews_target_live
    ON agent_reviews (target_agent_id, created_at DESC)
    WHERE hidden_at IS NULL;

-- Reviewer-side index for "reviews I've written".
CREATE INDEX IF NOT EXISTS idx_reviews_reviewer
    ON agent_reviews (reviewer_agent_id, created_at DESC);

-- ─── Tag join table ──────────────────────────────────────
--
-- Which of the target's capabilities the reviewer is attesting to.
-- Stored separately so a review can tag N capabilities; the
-- application requires ≥1 tag at write-time and verifies each tag is
-- both (a) a capability of the target and (b) a capability the
-- reviewer has actually invoked (relay_invocations.status != 'rejected').
--
-- ON DELETE CASCADE from both sides:
--   • review_id → if the review is removed (today only via target/
--     reviewer agent tombstone cascade), tags follow.
--   • capability_id → if a capability is deleted, its tag rows go;
--     the review survives (it may still tag other capabilities; the
--     application enforces ≥1 only at write-time, so a review that
--     ends up with zero tags after a cap delete is still legal — see
--     the "Tag rule" section of the spec).

CREATE TABLE IF NOT EXISTS agent_review_tags (
    review_id     UUID NOT NULL REFERENCES agent_reviews(id) ON DELETE CASCADE,
    capability_id UUID NOT NULL REFERENCES agent_capabilities(id) ON DELETE CASCADE,
    PRIMARY KEY (review_id, capability_id)
);

CREATE INDEX IF NOT EXISTS idx_review_tags_capability
    ON agent_review_tags (capability_id);
