-- Phase D1 (part 2 of 2). Separate from 0009 so the CHECK lands AFTER
-- any backfill of the agents.mode column.
--
-- Backfill runs first: every existing row that had a non-null
-- agent_card_url (push-style) MUST be flipped to mode='push' before
-- this CHECK is enforced. Today's data: scheduler-demo agents created
-- by examples/scheduler-demo/setup.py have no agent_card_url (the
-- column was added by 0009 with no value), so they default to
-- mode='pull' which already conforms. The UPDATE below is therefore
-- a no-op on a fresh DB but defensive on any DB where push-style
-- agents may have been written between 0009 and 0010 application.

UPDATE agents
   SET mode = 'push'
 WHERE agent_card_url IS NOT NULL
   AND mode = 'pull';

-- Card-or-pull invariant. After this CHECK, the schema guarantees:
--   mode='push' => agent_card_url IS NOT NULL
--   mode='pull' => agent_card_url IS NULL
-- A push agent without a canonical URL has no way to be invoked; a
-- pull agent with a URL would shadow our relay's published card URL.
ALTER TABLE agents
    DROP CONSTRAINT IF EXISTS agents_mode_card_consistency;
ALTER TABLE agents
    ADD CONSTRAINT agents_mode_card_consistency CHECK (
        (mode = 'push' AND agent_card_url IS NOT NULL) OR
        (mode = 'pull' AND agent_card_url IS NULL)
    );
