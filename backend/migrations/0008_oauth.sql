-- OAuth 2.1 authorization server tables.
--
-- The relay is an MCP server. MCP clients (Claude Desktop, Cursor, custom
-- agents) connect via OAuth 2.1 + PKCE. Most MCP hosts don't pre-register
-- with us — they self-register at runtime via RFC 7591, get a client_id,
-- then run the auth code flow.
--
-- Access tokens stay JWT (same shape as today's user JWTs), so the relay's
-- existing Bearer auth path validates them transparently. Auth codes are
-- short-lived (~10 minutes) and one-shot, so we store them here with a
-- code_hash (sha256) — never the plaintext code.

-- ─────────────────────────────────────────────────────────
-- oauth_clients — registered MCP hosts
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS oauth_clients (
    id UUID PRIMARY KEY,
    -- Public client_id (unguessable but not secret).
    client_id TEXT NOT NULL UNIQUE,
    -- Display name from the registration request, e.g. "Claude Desktop".
    client_name TEXT NOT NULL,
    -- Allowed redirect URIs (the IDP redirects the browser back to one of these).
    redirect_uris TEXT[] NOT NULL,
    -- Confidential clients have a hashed secret here. Public clients
    -- (Claude Desktop / Cursor / native apps) leave it null and rely
    -- on PKCE for security. Most MCP clients are public.
    client_secret_hash TEXT,
    -- Optional URL the client returned during registration.
    client_uri TEXT,
    -- Loose scope list — for v1 we only support 'relay.full'.
    scope TEXT NOT NULL DEFAULT 'relay.full',
    -- The user who registered this client (null when anonymous registration).
    created_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_oauth_clients_client_id ON oauth_clients (client_id);

-- ─────────────────────────────────────────────────────────
-- oauth_authorizations — pending and consumed authorization codes
-- ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS oauth_authorizations (
    id UUID PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- sha256 of the auth code we handed back to the client. The client
    -- presents the plaintext code on /oauth/token; we hash + lookup.
    code_hash TEXT NOT NULL UNIQUE,

    -- PKCE: client sends code_challenge (S256 of code_verifier) at /authorize,
    -- presents code_verifier at /token; we recompute and compare.
    code_challenge TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL DEFAULT 'S256'
        CHECK (code_challenge_method IN ('S256')),

    redirect_uri TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'relay.full',

    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_oauth_authorizations_code_hash ON oauth_authorizations (code_hash);
CREATE INDEX IF NOT EXISTS idx_oauth_authorizations_user ON oauth_authorizations (user_id, created_at DESC);
