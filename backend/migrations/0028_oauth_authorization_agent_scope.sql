-- Carry the chosen agent-management scope through the authorization-code
-- flow (PR3 of scoped-agent-grants).
--
-- The user picks the scope at consent time (POST /oauth/issue-code), but the
-- token — and therefore its `jti`, the key a credential_scopes row binds to
-- — isn't minted until POST /oauth/token. So we stash the choice on the
-- short-lived oauth_authorizations row and, at mint time, materialise it
-- into credential_scopes (migration 0027) inside the same transaction that
-- records `minted_jti`.
--
--   agent_scope        — 'all' (default / unrestricted), 'own', or 'selected'
--   selected_agent_ids — the explicit allow-list when agent_scope='selected';
--                        NULL / empty otherwise
--
-- 'all' is the default, so existing issue-code callers that don't send a
-- scope keep getting full access and no credential_scopes row is written
-- (resolve_grant treats "no row" as All).

ALTER TABLE oauth_authorizations
    ADD COLUMN IF NOT EXISTS agent_scope TEXT NOT NULL DEFAULT 'all'
        CHECK (agent_scope IN ('all', 'own', 'selected')),
    ADD COLUMN IF NOT EXISTS selected_agent_ids UUID[];
