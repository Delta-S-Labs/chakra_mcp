-- Credit ledger foundation, contract step: drop what the plan-tier quota
-- left behind. Design: docs/specs/2026-09-23-credit-ledger-foundation-design.md.
--
-- Credits replaced plans in #326; nothing reads `plans`, `accounts.plan_id`
-- or `usage_counters` any more. Forward-only: once this is applied, an older
-- binary refuses to boot (it doesn't know this migration), and the plan tiers
-- and old monthly counters are gone (`relay_invocations` keeps the history).
--
-- Locking: every drop needs ACCESS EXCLUSIVE on `accounts` (`usage_counters`
-- too — its foreign key's triggers live on `accounts`), and `accounts` is
-- read on the invocation path. A lock request that *waits* queues every later
-- reader behind it, so never wait long: try for 100 ms, and on a timeout back
-- off (letting the queued readers through) and retry. Once the lock is held,
-- the drops are catalog-only and take milliseconds.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE accounts IN ACCESS EXCLUSIVE MODE;
            ALTER TABLE accounts DROP COLUMN IF EXISTS plan_id;
            DROP TABLE IF EXISTS usage_counters;
            DROP TABLE IF EXISTS plans;
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
