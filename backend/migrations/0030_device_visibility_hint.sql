-- Let the pairing client hint the agent's visibility too, alongside the
-- slug / display_name / description hints it already sends (migration 0014).
-- The consent form pre-fills it and the user can still edit before approving.
--
-- Constrained to the same set the device-approve endpoint accepts today
-- (private | network); NULL means "no hint — fall back to the default".
ALTER TABLE oauth_device_codes
    ADD COLUMN IF NOT EXISTS agent_visibility_hint TEXT
        CHECK (agent_visibility_hint IS NULL OR agent_visibility_hint IN ('private', 'network'));
