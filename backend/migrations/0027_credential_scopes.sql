-- Per-credential agent-management scope (PR2 of scoped-agent-grants).
--
-- A scope row, keyed by the credential that will present it (an OAuth /
-- device JWT by its `jti`, or an API key by its id), narrows what that
-- credential may MANAGE:
--
--   'all'      → every agent in accounts the caller is a member of — the
--                pre-existing behaviour, and the IMPLICIT default whenever
--                no row exists, so every credential minted before this
--                feature keeps working unchanged.
--   'own'      → only agents this credential's issuer created, matched via
--                agents.created_by_client_id / created_by_api_key_id
--                (migration 0026).
--   'selected' → only the agents enumerated in credential_scope_agents.
--
-- Enforcement layers ON TOP of account membership — a scope can never
-- widen access beyond the accounts the caller already belongs to; it only
-- ever narrows. Nothing writes these rows yet (that lands in PR3, the
-- OAuth scope grammar); until then no row exists ⇒ everyone is 'all' ⇒ no
-- behaviour change.

CREATE TABLE IF NOT EXISTS credential_scopes (
    id          UUID PRIMARY KEY,

    -- Which credential this scope binds to:
    --   'jwt'     → cred_ref = the token's jti (UserClaims.jti) as text
    --   'api_key' → cred_ref = api_keys.id as text
    cred_kind   TEXT NOT NULL CHECK (cred_kind IN ('jwt', 'api_key')),
    cred_ref    TEXT NOT NULL,

    -- The OAuth client that obtained this grant (NULL for raw API keys).
    -- Denormalised from the auth/device flow so an 'own' check doesn't have
    -- to re-resolve it from the jti lineage on every call.
    client_id   TEXT REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    agent_scope TEXT NOT NULL CHECK (agent_scope IN ('all', 'own', 'selected')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (cred_kind, cred_ref)
);

CREATE INDEX IF NOT EXISTS idx_credential_scopes_lookup
    ON credential_scopes (cred_kind, cred_ref);

-- Explicit allow-list for agent_scope='selected'. Also where a newly
-- created agent is added when an app holding a 'selected' grant creates
-- one, so it can keep managing what it just made.
CREATE TABLE IF NOT EXISTS credential_scope_agents (
    credential_scope_id UUID NOT NULL
        REFERENCES credential_scopes(id) ON DELETE CASCADE,
    agent_id            UUID NOT NULL
        REFERENCES agents(id) ON DELETE CASCADE,
    added_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (credential_scope_id, agent_id)
);
