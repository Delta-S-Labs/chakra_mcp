-- Owner consent, PR 1: per-capability consent settings (spec §3, §9).
-- Design: docs/superpowers/specs/2026-10-23-owner-consent-design.md.
--
-- Locking: adding columns and constraints needs ACCESS EXCLUSIVE on
-- `agent_capabilities`, which every invocation reads. A lock request that
-- *waits* queues every later reader behind it, so never wait long: try for
-- 100 ms, and on a timeout back off (letting the queued readers through) and
-- retry, as 0036 does. Once held, the work is catalog-only: the defaults are
-- constants, so nothing is rewritten, and the CHECKs are added NOT VALID (no
-- scan). 0043 validates them without blocking reads or writes.

DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE agent_capabilities IN ACCESS EXCLUSIVE MODE;
            ALTER TABLE agent_capabilities
                ADD COLUMN IF NOT EXISTS consent_required BOOLEAN NOT NULL DEFAULT false,
                ADD COLUMN IF NOT EXISTS consent_approvers UUID[] NOT NULL DEFAULT '{}',
                ADD COLUMN IF NOT EXISTS consent_max_validity_secs INT NOT NULL DEFAULT 0,
                ADD COLUMN IF NOT EXISTS consent_wait_secs INT NOT NULL DEFAULT 86400;
            ALTER TABLE agent_capabilities
                ADD CONSTRAINT agent_capabilities_consent_max_validity_check
                    CHECK (consent_max_validity_secs IN (0, 3600, 86400, 604800, 2592000)) NOT VALID,
                ADD CONSTRAINT agent_capabilities_consent_wait_check
                    CHECK (consent_wait_secs BETWEEN 300 AND 604800) NOT VALID,
                ADD CONSTRAINT agent_capabilities_consent_approvers_check
                    CHECK (cardinality(consent_approvers) <= 20) NOT VALID;
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
