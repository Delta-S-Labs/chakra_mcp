-- Credits hardening, applied before the async swap (PR2) starts writing
-- these tables. Design: docs/specs/2026-09-23-credit-ledger-foundation-design.md.
--
-- Additive: one storage setting and two CHECKs. 0036 later drops `plans`,
-- `accounts.plan_id` and `usage_counters`.

SET LOCAL lock_timeout = '5s';

-- The worker drains the queue to empty every few seconds, so each VACUUM
-- would find its trailing pages empty and truncate them. Truncating takes an
-- ACCESS EXCLUSIVE lock, and the invocation path's queue inserts would wait
-- behind it. Keep the pages instead; the next inserts reuse them.
ALTER TABLE credit_charge_queue SET (vacuum_truncate = false);

-- Guard the per-account overrides before anything writes them (NULL still
-- means "use the default"): a rate limit of 0 or less would refuse every
-- call, and a negative grant would debit the account every month.
ALTER TABLE credit_wallets
    ADD CONSTRAINT credit_wallets_rate_limit_positive
        CHECK (rate_limit_per_min > 0),
    ADD CONSTRAINT credit_wallets_monthly_grant_non_negative
        CHECK (monthly_free_grant_mc >= 0);
