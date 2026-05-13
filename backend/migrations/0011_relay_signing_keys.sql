-- Phase D2b: Ed25519 signing keys for Agent Card JWS signatures.
--
-- Schema for the relay's own signing key material. We mint short-
-- lived JWS-shaped signatures over every public Agent Card we
-- publish; verifiers (generic A2A clients) fetch our public keys
-- from /.well-known/jwks.json and verify against the kid in the
-- card's signature.protected header.
--
-- Key lifecycle:
--   active   — newest key. Used to sign all NEW cards.
--   retired  — past its rotation date. Still in JWKS during the
--              overlap window so previously-signed cards keep
--              verifying.
--   expired  — past the overlap window. Removed from JWKS.
--              Soft-deleted (kept in table for forensics).
--
-- v1 simplification: private_key_bytes is stored unencrypted in this
-- table. Encryption-at-rest (KMS-wrapped or pgcrypto) is documented
-- in the discovery spec as a follow-up. The DB password + Postgres
-- TLS bound the threat model for v1.

CREATE TABLE IF NOT EXISTS relay_signing_keys (
    -- The kid that appears in JWS protected headers and in JWKS.
    -- UUID v7 keeps it sortable by issue order.
    kid UUID PRIMARY KEY,

    -- Ed25519 keys are exactly 32 bytes each.
    private_key_bytes BYTEA NOT NULL CHECK (octet_length(private_key_bytes) = 32),
    public_key_bytes  BYTEA NOT NULL CHECK (octet_length(public_key_bytes)  = 32),

    -- When this key was minted. Most recent non-expired key is the
    -- one we sign new cards with.
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- When the key was retired from active signing duty. Still in
    -- JWKS until expires_at.
    retired_at TIMESTAMPTZ,

    -- When the key falls out of JWKS entirely. NULL = still
    -- verifiable; cards signed under this key can still be checked.
    expires_at TIMESTAMPTZ,

    -- Loose audit field — operator can note rotation reason.
    rotation_reason TEXT,

    CHECK (retired_at IS NULL OR retired_at >= created_at),
    CHECK (expires_at IS NULL OR retired_at IS NOT NULL),
    CHECK (expires_at IS NULL OR expires_at >= retired_at)
);

-- "Which kid am I signing with right now?" — the most recent key
-- with retired_at IS NULL. Cheap index for the hot path.
CREATE INDEX IF NOT EXISTS idx_relay_signing_keys_active
    ON relay_signing_keys (created_at DESC)
    WHERE retired_at IS NULL;

-- "Which keys belong in JWKS right now?" — anything not fully
-- expired. Postgres rejects partial-index predicates that reference
-- non-IMMUTABLE functions like now(), so this is a regular btree
-- and the JWKS handler filters at query time:
--    WHERE expires_at IS NULL OR expires_at > now()
CREATE INDEX IF NOT EXISTS idx_relay_signing_keys_jwks
    ON relay_signing_keys (created_at DESC, expires_at);
