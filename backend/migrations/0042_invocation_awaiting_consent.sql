-- Owner consent, PR 1: the `awaiting_consent` invocation status (spec §4, §9).
-- Design: docs/superpowers/specs/2026-10-23-owner-consent-design.md.
--
-- A request to a capability that asks first is held as `awaiting_consent`
-- until an approver decides. The inbox claims only `pending`, so held rows
-- are never delivered.
--
-- Locking: as 0041, on `relay_invocations` (written by every invocation).
-- The replacement CHECK is added NOT VALID, so swapping it is catalog-only;
-- new rows are checked at once, and 0043 validates existing rows.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE relay_invocations IN ACCESS EXCLUSIVE MODE;
            ALTER TABLE relay_invocations
                DROP CONSTRAINT IF EXISTS relay_invocations_status_check,
                ADD CONSTRAINT relay_invocations_status_check
                    CHECK (status IN ('pending', 'in_progress', 'rejected', 'succeeded',
                                      'failed', 'timeout', 'awaiting_consent')) NOT VALID;
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
