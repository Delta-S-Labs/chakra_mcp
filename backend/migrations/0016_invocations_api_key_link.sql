-- Record which credential (if any) authorised each invocation.
--
-- The auth extractor today picks between three pathways:
--   * user JWT  → revoked_tokens.jti is the per-token id
--   * API key   → api_keys row identifies the credential
--   * (no auth) → reaches the relay over the inbox bridge, never hits
--                  /v1/invoke directly
--
-- For the /v1/api-keys/{id}/usage endpoint to attribute traffic to a
-- specific key we need the FK here. Existing rows stay NULL — they
-- predate the link. ON DELETE SET NULL so revoking an old key doesn't
-- erase its audit trail.

ALTER TABLE relay_invocations
    ADD COLUMN IF NOT EXISTS api_key_id UUID
        REFERENCES api_keys(id) ON DELETE SET NULL;

-- Filtered index — the per-key dashboard joins on this column and
-- always filters out the NULL pre-link rows. Keeps the index small.
CREATE INDEX IF NOT EXISTS idx_invocations_api_key
    ON relay_invocations (api_key_id, created_at DESC)
    WHERE api_key_id IS NOT NULL;
