-- Plans (tiers) + the per-account usage substrate for rate limiting and
-- monthly invocation quotas. Design: docs/specs/2026-07-23-usage-quotas-
-- rate-limiting-design.md.
--
-- This migration is deliberately non-breaking on its own: it adds a plan to
-- every account (defaulting to `free`) and a monthly counter table, but wires
-- no enforcement. The relay only reads these once PR 3 lands, so PR 1 can
-- deploy ahead of the binary that uses it.
--
-- `accounts.plan_id` is added as NOT NULL DEFAULT <free> in a single ALTER,
-- which backfills every existing row AND auto-assigns new accounts — so the
-- existing account-creation INSERTs (which don't mention plan_id) keep working
-- with no code change. The default references a fixed sentinel UUID so the
-- column DEFAULT can be a constant.

-- ─── plans ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS plans (
    id                       UUID PRIMARY KEY,
    name                     TEXT NOT NULL UNIQUE,          -- 'free' | 'pro' | 'enterprise'
    rate_limit_per_min       INTEGER NOT NULL,             -- invocations/min per account
    monthly_invocation_quota BIGINT,                       -- NULL = unlimited
    is_default               BOOLEAN NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- At most one default plan (the tier new accounts land on).
CREATE UNIQUE INDEX IF NOT EXISTS plans_one_default
    ON plans (is_default) WHERE is_default;

-- Seed with fixed sentinel UUIDs so accounts.plan_id can carry a constant
-- DEFAULT. Numbers are placeholders — tune before enforcement is turned on.
INSERT INTO plans (id, name, rate_limit_per_min, monthly_invocation_quota, is_default) VALUES
    ('00000000-0000-0000-0000-0000000000f1', 'free',       60,   1000,  TRUE),
    ('00000000-0000-0000-0000-0000000000f2', 'pro',        600,  50000, FALSE),
    ('00000000-0000-0000-0000-0000000000f3', 'enterprise', 6000, NULL,  FALSE)
ON CONFLICT (name) DO NOTHING;

-- ─── accounts.plan_id ────────────────────────────────────
-- NOT NULL DEFAULT free: backfills existing rows and auto-assigns new ones.
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS plan_id UUID NOT NULL
    DEFAULT '00000000-0000-0000-0000-0000000000f1' REFERENCES plans(id);

-- ─── usage_counters — durable monthly quota ledger ───────
-- One row per (account, calendar month). Incremented once per invocation in
-- PR 3; read at the policy gate to check the monthly quota. period_start is
-- date_trunc('month', now() AT TIME ZONE 'UTC')::date.
CREATE TABLE IF NOT EXISTS usage_counters (
    account_id   UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    period_start DATE NOT NULL,
    invocations  BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, period_start)
);
