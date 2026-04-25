-- Phase 1.0 — users, accounts (orgs), memberships, oauth provider links, API keys.
--
-- Convention: Postgres-managed UUIDs via uuidv7-style sortable IDs are
-- generated in app code; a few utility tables use gen_random_uuid().

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ─────────────────────────────────────────────────────────
-- users — humans on the network
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    avatar_url TEXT,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    -- For email/password auth (later phase). Null for OAuth-only users.
    password_hash TEXT,
    -- Email verification (set when user verifies via OOB link).
    email_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users (LOWER(email));

-- ─────────────────────────────────────────────────────────
-- oauth_links — providers attached to a user
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS oauth_links (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,                 -- 'github' | 'google' | future
    provider_user_id TEXT NOT NULL,         -- the provider's stable ID
    raw_profile JSONB,                      -- whatever the provider returned, for audit
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, provider_user_id)
);

CREATE INDEX IF NOT EXISTS idx_oauth_links_user ON oauth_links (user_id);

-- ─────────────────────────────────────────────────────────
-- accounts — what the build spec calls accounts; in product
-- terms these are organizations. A user always has a default
-- personal account; teams create additional org accounts.
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS accounts (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,              -- url-safe handle, e.g. "delta-s-labs"
    display_name TEXT NOT NULL,
    account_type TEXT NOT NULL CHECK (account_type IN ('individual', 'organization')),
    -- For individual accounts: the human who owns it. For orgs: who created it.
    owner_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_accounts_owner ON accounts (owner_user_id);

-- ─────────────────────────────────────────────────────────
-- account_memberships — humans inside an account
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS account_memberships (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    invited_by UUID REFERENCES users(id) ON DELETE SET NULL,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_memberships_user ON account_memberships (user_id);
CREATE INDEX IF NOT EXISTS idx_memberships_account ON account_memberships (account_id);

-- ─────────────────────────────────────────────────────────
-- account_invites — pending email invites to an account
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS account_invites (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    invited_by UUID NOT NULL REFERENCES users(id),
    token_hash TEXT NOT NULL,               -- sha256 of the actual token
    accepted_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, email)
);

CREATE INDEX IF NOT EXISTS idx_invites_email ON account_invites (LOWER(email));

-- ─────────────────────────────────────────────────────────
-- api_keys — personal access tokens for a user
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS api_keys (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Optional account scope. NULL = full user scope; non-null = scoped to one account.
    account_id UUID REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- Only the SHA-256 hash of the key is stored. The plaintext is shown
    -- exactly once on creation.
    key_hash TEXT NOT NULL UNIQUE,
    -- A short identifying prefix that is safe to display (e.g. "ck_live_a1b2…").
    key_prefix TEXT NOT NULL,
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_api_keys_user ON api_keys (user_id);
CREATE INDEX IF NOT EXISTS idx_api_keys_account ON api_keys (account_id);

-- updated_at maintenance trigger
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS users_updated_at ON users;
CREATE TRIGGER users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

DROP TRIGGER IF EXISTS accounts_updated_at ON accounts;
CREATE TRIGGER accounts_updated_at BEFORE UPDATE ON accounts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
