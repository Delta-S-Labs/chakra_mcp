-- Server-side JWT revocation. Issued JWTs are stateless and cannot
-- be deactivated by clearing a cookie alone — if the plaintext token
-- has been exfiltrated, the only way to kill it is to record the
-- revocation here and have the JWT-decode middleware check this
-- table before trusting the claim.
--
-- One row per revoked token. Indexed on (jti) for the validation
-- hot path; (expires_at) so the GC sweep is cheap.
CREATE TABLE IF NOT EXISTS revoked_tokens (
    jti UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    revoked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Mirror the original JWT's `exp` so a periodic sweep can prune
    -- rows once natural expiry has passed.
    expires_at TIMESTAMPTZ NOT NULL,
    -- Optional: the reason / source of the revocation. Free text;
    -- we'll seed with values like 'user_signout', 'admin_revoke',
    -- 'session_rotation' — useful for audit later.
    reason TEXT
);

CREATE INDEX IF NOT EXISTS idx_revoked_tokens_expires_at
    ON revoked_tokens (expires_at);
