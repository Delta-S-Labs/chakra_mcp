-- Credits P4: buying credits through Dodo Payments.
-- docs/superpowers/specs/2026-10-06-credits-p4-purchasing-design.md §4
--
-- One row per checkout our server creates, before the buyer sees Dodo's
-- checkout. The webhook credits a payment only through its checkout session
-- (`dodo_session_id`), and only once (`dodo_payment_id`, plus the ledger's
-- `credit_ledger_purchase_ref_uniq` from 0034).
--
-- No foreign keys: like `credit_ledger`, a row outlives a deleted org or
-- user, and creating this table takes no lock on `users` or `accounts`.

CREATE TABLE credit_checkouts (
    id               UUID        PRIMARY KEY,
    account_id       UUID        NOT NULL,
    -- The buyer, and their email when they bought (for the payments list).
    user_id          UUID        NOT NULL,
    buyer_email      TEXT        NOT NULL,
    amount_cents     INTEGER     NOT NULL CHECK (amount_cents > 0),
    currency         TEXT        NOT NULL DEFAULT 'USD',
    -- Fixed when the checkout is created: a later price change doesn't touch it.
    credits_mc       BIGINT      NOT NULL CHECK (credits_mc > 0),
    -- open → paid | failed | unapplied. A failed checkout can still be paid
    -- (a declined card, then a retry in the same Dodo session). Reads show an
    -- `open` row older than 24 hours as expired; nothing stores that.
    status           TEXT        NOT NULL DEFAULT 'open'
                     CHECK (status IN ('open', 'paid', 'failed', 'unapplied')),
    -- Paid but not credited, for the operator: the account was deleted, or
    -- the payment didn't match the checkout (a discount, another currency,
    -- less than the amount).
    unapplied_reason TEXT        CHECK (unapplied_reason IN ('account_gone', 'not_as_agreed')),
    dodo_session_id  TEXT        UNIQUE,
    dodo_payment_id  TEXT        UNIQUE,
    invoice_url      TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at          TIMESTAMPTZ,
    CHECK (status <> 'paid' OR (dodo_payment_id IS NOT NULL AND paid_at IS NOT NULL)),
    CHECK ((status = 'unapplied') = (unapplied_reason IS NOT NULL))
);

-- The payments list (newest first) and the per-account hourly throttle.
CREATE INDEX credit_checkouts_account_created ON credit_checkouts (account_id, created_at DESC);
