-- Waking pull agents W4: index open deadlines and live leases for the
-- sweep (design §10, §12).
--
-- `relay_invocations_open_expiry` keeps NULLs, so the sweep's backfill of
-- rows from before 0053 uses it too and costs nothing once they're done.
--
-- Built normally, not CONCURRENTLY, for 0047's reasons: a normal build holds
-- SHARE, so writes wait during the scan and reads don't. The lock is taken
-- with 0047's retry, so it never queues behind a long transaction.
-- Production's table held 441 rows on 2026-10-25 (see 0047).

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE relay_invocations IN SHARE MODE;
            CREATE INDEX IF NOT EXISTS relay_invocations_open_expiry
                ON relay_invocations (expires_at)
                WHERE status IN ('pending', 'in_progress');
            CREATE INDEX IF NOT EXISTS relay_invocations_live_lease
                ON relay_invocations (lease_until)
                WHERE status = 'in_progress' AND lease_until IS NOT NULL;
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
