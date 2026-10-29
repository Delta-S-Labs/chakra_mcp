-- Waking pull agents W4: claims that go stale, and deadlines (design §9,
-- §10, §12). Design: docs/superpowers/specs/2026-10-28-waking-pull-agents-design.md.
--
-- `lease_until`: when an `in_progress` claim lapses and the call goes back
-- to `pending` (NULL for `human_in_loop` calls, which don't lapse).
-- `deliveries`: how many times it was claimed; the third lapse times it out.
-- `expires_at`: when an open call times out. Rows from before this
-- migration get one from the sweep.
--
-- Nullable columns and a constant default only change the catalog, so the
-- table isn't rewritten. Locking as 0046: `relay_invocations` is written by
-- every invocation, so the lock is taken in a short retry loop and never
-- queues behind a long transaction. The indexes come in 0054.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE relay_invocations IN ACCESS EXCLUSIVE MODE;
            ALTER TABLE relay_invocations
                ADD COLUMN IF NOT EXISTS lease_until TIMESTAMPTZ,
                ADD COLUMN IF NOT EXISTS deliveries INTEGER NOT NULL DEFAULT 0,
                ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;
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
