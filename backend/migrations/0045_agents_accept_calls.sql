-- Calls, C1: who may call an agent (spec §3.1).
-- Design: docs/superpowers/specs/2026-10-25-calls-design.md.
--
-- `friends` (the default), `anyone` or `nobody`, set by the agent's owner.
--
-- Locking: `agents` is read on every invocation, and a lock request that
-- waits queues every later reader behind it. So, as 0036: try for 100 ms,
-- and on a timeout back off and retry. A column with a constant default is
-- catalog-only, and the CHECK is added NOT VALID; 0048 validates it.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE agents IN ACCESS EXCLUSIVE MODE;
            ALTER TABLE agents
                ADD COLUMN IF NOT EXISTS accept_calls TEXT NOT NULL DEFAULT 'friends';
            ALTER TABLE agents
                DROP CONSTRAINT IF EXISTS agents_accept_calls_check,
                ADD CONSTRAINT agents_accept_calls_check
                    CHECK (accept_calls IN ('friends', 'anyone', 'nobody')) NOT VALID;
            EXIT;
        EXCEPTION WHEN lock_not_available OR deadlock_detected THEN
            attempt := attempt + 1;
            IF attempt >= 50 THEN
                RAISE;
            END IF;
            PERFORM pg_sleep(LEAST(0.05 * attempt, 1.0));
        END;
    END LOOP;
END $$;
