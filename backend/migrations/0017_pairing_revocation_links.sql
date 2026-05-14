-- Pairing-session unification — make device-flow and authorization-code
-- rows independently revocable, and let us tie each back to the JWT it
-- minted so the existing `revoked_tokens` path can kill the live token.
--
-- The unified GET /v1/pairings view treats device codes and oauth
-- authorisations as "paired sessions". Each pathway gets:
--   * revoked_at — flips a session off without waiting for natural
--                  expiry. The token endpoint refuses to mint if set.
--   * minted_jti — the JWT id we handed out from this session. NULL
--                  for sessions that never produced a token. The
--                  revoke endpoint inserts into revoked_tokens with
--                  this jti so the live token stops authenticating.

ALTER TABLE oauth_device_codes
    ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ;
ALTER TABLE oauth_device_codes
    ADD COLUMN IF NOT EXISTS minted_jti UUID;

ALTER TABLE oauth_authorizations
    ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ;
ALTER TABLE oauth_authorizations
    ADD COLUMN IF NOT EXISTS minted_jti UUID;
