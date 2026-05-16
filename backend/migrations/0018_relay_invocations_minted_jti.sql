-- Record which JWT (if any) authorised each invocation, so we can roll
-- usage up by *paired session* — the device-flow or auth-code grant the
-- token was minted from.
--
-- The relay's auth extractor already picks between two pathways:
--   * user JWT  → revoked_tokens.jti is the per-token id; we capture it
--                  here. Migration 0017 stamped that same jti on the
--                  `oauth_device_codes` / `oauth_authorizations` row that
--                  minted the token, so a join on `minted_jti` gives us
--                  "which pair did this call go through?".
--   * API key   → the `api_key_id` column from migration 0016 already
--                  attributes the row.
--
-- Existing pre-migration rows have `minted_jti IS NULL` — same shape as
-- migration 0016's `api_key_id`. ON DELETE is not relevant: jtis are
-- standalone identifiers, not FKs into any retained table.

ALTER TABLE relay_invocations
    ADD COLUMN IF NOT EXISTS minted_jti UUID;

-- Filtered index — the per-pair dashboard joins on this column and
-- always filters out the NULL pre-link rows. Keeps the index small.
CREATE INDEX IF NOT EXISTS idx_invocations_minted_jti
    ON relay_invocations (minted_jti, created_at DESC)
    WHERE minted_jti IS NOT NULL;
