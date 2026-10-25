-- Calls, C1: the call an invocation was made in (spec §7, §13).
-- Design: docs/superpowers/specs/2026-10-25-calls-design.md.
--
-- Nullable, no default and no FK (like invocation_charges), so adding it is
-- catalog-only. Locking as 0045: `relay_invocations` is written by every
-- invocation. The index comes in 0047, in its own transaction, so this
-- lock isn't held through the index build.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE relay_invocations IN ACCESS EXCLUSIVE MODE;
            ALTER TABLE relay_invocations ADD COLUMN IF NOT EXISTS call_id UUID;
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
