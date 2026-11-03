-- Index charges by time for Grafana's Revenue dashboard, whose spend panels
-- filter every account's charges on `charged_at` alone; the only index so
-- far leads with `account_id` (0034), so they scan the table.
--
-- Built normally, not CONCURRENTLY, for 0047's reasons: a normal build holds
-- SHARE, so writes wait during the scan and reads don't. Only the credits
-- worker writes this table, off the invocation path. The lock is taken with
-- 0047's retry, so it never queues behind a long transaction. Production's
-- table held 6 rows (40 kB) on 2026-11-03.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE invocation_charges IN SHARE MODE;
            CREATE INDEX IF NOT EXISTS invocation_charges_charged_at_idx
                ON invocation_charges (charged_at);
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
