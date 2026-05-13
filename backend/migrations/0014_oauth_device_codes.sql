-- OAuth 2.1 Device Authorization Grant (RFC 8628).
--
-- Powers the agent-onboarding device flow:
--
--   1. Agent calls POST /oauth/device_authorization with no credentials,
--      gets back a (device_code, user_code, verification_uri_complete).
--   2. Agent prints the verification URL and starts polling /oauth/token.
--   3. Human opens the URL in their browser, signs in to chakramcp.com,
--      picks an agent slug + display name, clicks "Approve".
--   4. Frontend POSTs to /oauth/device-approve which marks the row and
--      attaches the agent_id.
--   5. Next agent poll on /oauth/token sees the approval and returns
--      an access_token + the freshly-created agent record details.
--
-- One-shot: once consumed, the row stays for audit but can't issue
-- another token. Rows older than expires_at are purged by a periodic
-- job (kept here unindexed; the table is small).

CREATE TABLE IF NOT EXISTS oauth_device_codes (
    id UUID PRIMARY KEY,

    -- Optional registered client. Device flow allows the "no client" case
    -- (agent has nothing pre-registered) — we mint an ad-hoc client_id
    -- per-session in that path.
    client_id TEXT REFERENCES oauth_clients(client_id) ON DELETE CASCADE,

    -- sha256 of the device_code we returned to the agent.
    -- The agent presents the plaintext on /oauth/token; we hash + lookup.
    device_code_hash TEXT NOT NULL UNIQUE,

    -- Short human-readable code. ALSO doubles as the session id in the
    -- verification_uri_complete (we hand the user a clickable URL rather
    -- than asking them to type 'ABCD-1234'). Indexed for /onboard lookup.
    user_code TEXT NOT NULL UNIQUE,

    scope TEXT NOT NULL DEFAULT 'relay.full',

    -- Hints from the agent during /device_authorization that the frontend
    -- pre-fills on the consent page. None of these are trusted — the
    -- user can override on the consent screen.
    persona TEXT,
    agent_slug_hint TEXT,
    agent_display_name_hint TEXT,
    agent_description_hint TEXT,

    -- Filled by /oauth/device-approve once the user approves.
    approved_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    approved_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    approved_at TIMESTAMPTZ,

    -- Denied path. Lets the agent stop polling immediately on next tick.
    denied_at TIMESTAMPTZ,

    expires_at TIMESTAMPTZ NOT NULL,

    -- RFC 8628 §3.5 polling guidance. We bump this when the agent polls
    -- faster than allowed.
    poll_interval_seconds INTEGER NOT NULL DEFAULT 5,
    last_polled_at TIMESTAMPTZ,

    -- One-shot: once we issue a token against this device_code, mark used.
    consumed_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_oauth_device_codes_device_code_hash
    ON oauth_device_codes (device_code_hash);
CREATE INDEX IF NOT EXISTS idx_oauth_device_codes_user_code
    ON oauth_device_codes (user_code);
CREATE INDEX IF NOT EXISTS idx_oauth_device_codes_approved_user
    ON oauth_device_codes (approved_user_id, approved_at DESC);
