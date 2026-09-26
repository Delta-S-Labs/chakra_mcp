-- Credit ledger foundation, expand step (Phase 1 of quota productization).
-- Design: docs/specs/2026-09-23-credit-ledger-foundation-design.md.
--
-- Credits replace the per-plan monthly invocation quota. Accounting is fully
-- asynchronous: the invocation path only writes one row to
-- `credit_charge_queue` (in the same statement as its `relay_invocations`
-- row) and reads in-memory switches; a background worker charges, grants,
-- and decides who is blocked.
--
-- Additive only. It touches neither `relay_invocations` nor `accounts`, so
-- the relay still running the plan-quota path during the deploy's migrate
-- step is unaffected. The switch to credits ships as code with no migration;
-- 0035 later drops `plans`, `accounts.plan_id` and `usage_counters`.
--
-- No backfill: a wallet is created at an account's first charge, so
-- never-used accounts don't accrue free credits.

SET LOCAL lock_timeout = '5s';

-- The credit system has no notion of plan tiers, so an account on `pro` or
-- `enterprise` would silently lose its tier. Refuse to run rather than
-- downgrade anyone.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM accounts a
          JOIN plans p ON p.id = a.plan_id
         WHERE p.name <> 'free'
    ) THEN
        RAISE EXCEPTION 'credits migration: some accounts are on a non-free plan; '
                        'give them credit wallets before migrating';
    END IF;
END $$;

-- ─── credit_wallets ──────────────────────────────────────
-- One per account: the balance plus per-account overrides of the global
-- defaults (NULL = use the default).
CREATE TABLE credit_wallets (
    account_id            UUID        PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    balance_mc            BIGINT      NOT NULL DEFAULT 0,   -- milli-credits; may dip below 0
    monthly_free_grant_mc BIGINT,                           -- NULL → CREDITS_DEFAULT_MONTHLY_FREE_MC
    rate_limit_per_min    INTEGER,                          -- NULL → LIMITS_DEFAULT_RATE_PER_MIN
    free_grant_period     DATE,                             -- first-of-month of the last grant; NULL = never granted
    unlimited             BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now() -- set explicitly by writers
);
CREATE INDEX credit_wallets_grant_period_idx ON credit_wallets (free_grant_period);

-- ─── credit_ledger ───────────────────────────────────────
-- Append-only record of credits in and adjustments. Consumption lives in
-- `invocation_charges` instead. No FK to accounts: an org hard-delete must
-- not erase purchase history or the dedupe keys below.
CREATE TABLE credit_ledger (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id       UUID        NOT NULL,
    delta_mc         BIGINT      NOT NULL,
    reason           TEXT        NOT NULL
                     CHECK (reason IN ('free_grant', 'purchase', 'admin_grant', 'adjustment')),
    external_ref     TEXT,                                  -- e.g. a Dodo payment id
    balance_after_mc BIGINT      NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    metadata         JSONB,                                 -- acting admin, note, grant period
    -- A NULL ref would slip past the unique index below.
    CHECK (reason <> 'purchase' OR external_ref IS NOT NULL)
);
CREATE INDEX credit_ledger_account_created_idx ON credit_ledger (account_id, created_at DESC);
-- A retried payment webhook can never credit the same payment twice.
CREATE UNIQUE INDEX credit_ledger_purchase_ref_uniq ON credit_ledger (external_ref)
    WHERE reason = 'purchase';

-- ─── credit_charge_queue ─────────────────────────────────
-- The invocation path's only credit write. No FKs, to keep that insert
-- lean; the worker drains it every few seconds. Rows are inserted then
-- deleted, so vacuum on a fixed dead-tuple count rather than a fraction of
-- the (always small) table.
CREATE TABLE credit_charge_queue (
    invocation_id UUID PRIMARY KEY,
    account_id    UUID NOT NULL
) WITH (autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);

-- ─── invocation_charges ──────────────────────────────────
-- One row per charged invocation; the primary key makes double-charging
-- impossible. No FKs: billing history outlives deleted accounts and the
-- `relay_invocations` audit table.
CREATE TABLE invocation_charges (
    invocation_id UUID        PRIMARY KEY,
    account_id    UUID        NOT NULL,
    cost_mc       BIGINT      NOT NULL,
    charged_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invocation_charges_account_idx ON invocation_charges (account_id, charged_at DESC);
