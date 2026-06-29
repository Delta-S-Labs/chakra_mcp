-- Carry the chosen agent-management scope through the device flow (PR3c of
-- scoped-agent-grants), mirroring 0028 for the authorization-code flow.
--
-- The user picks the scope at /oauth/device-approve; the token (and its jti,
-- the key a credential_scopes row binds to) isn't minted until
-- /oauth/token, so stash the choice on the device-code row and materialise
-- it at mint time (migration 0027).
--
-- Device nuance: an unregistered device session has a NULL client_id, so an
-- 'own' grant can't be client-matched (own = agents.created_by_client_id =
-- the credential's client). At mint time that case is rewritten to a
-- 'selected' scope over the agent the pairing produced — so device 'own'
-- always means "the agent(s) this pairing created", never "nothing".

ALTER TABLE oauth_device_codes
    ADD COLUMN IF NOT EXISTS agent_scope TEXT NOT NULL DEFAULT 'all'
        CHECK (agent_scope IN ('all', 'own', 'selected')),
    ADD COLUMN IF NOT EXISTS selected_agent_ids UUID[];
