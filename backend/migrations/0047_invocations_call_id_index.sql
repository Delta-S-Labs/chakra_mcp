-- Calls, C1: index the invocations made in calls (spec §13).
-- Design: docs/superpowers/specs/2026-10-25-calls-design.md.
--
-- Built normally, not CONCURRENTLY: sqlx runs a `-- no-transaction` file as
-- one implicit transaction, so a concurrent build can't be paired with a
-- recovery step, and an invalid index left by a failed one would pass
-- IF NOT EXISTS unnoticed. A normal build holds SHARE: writes to
-- relay_invocations wait during the scan, reads don't. Every existing row
-- has a null call_id, and production's table held 441 rows (1.5 MB) on
-- 2026-10-25. The lock is taken with 0045's retry, so it never queues
-- behind a long transaction.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE relay_invocations IN SHARE MODE;
            CREATE INDEX IF NOT EXISTS relay_invocations_call_id
                ON relay_invocations (call_id) WHERE call_id IS NOT NULL;
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
