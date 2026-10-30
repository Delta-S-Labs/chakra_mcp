-- Waking pull agents W6: wake URLs (design §7, §12). Design:
-- docs/superpowers/specs/2026-10-28-waking-pull-agents-design.md.
--
-- A pull agent may give a URL the relay pings when work arrives for it, at
-- most every 30 s: JSON signed with `secret`, or a plain line of text (an
-- ntfy topic, a chat bot). The URL may hold a bot token, so only the agent's
-- account reads it, and the secret is shown once, when the URL is set.
-- After 20 failures in a row the URL is switched off (`disabled_at`).
--
-- A new table, so the hot `agents` table isn't altered. No foreign key, as
-- 0039 explains: creating one would lock `agents`; a row for a deleted
-- agent is harmless, since readers join through `agents`.
CREATE TABLE agent_wake (
    agent_id     UUID PRIMARY KEY,
    url          TEXT NOT NULL,
    format       TEXT NOT NULL DEFAULT 'json' CHECK (format IN ('json', 'text')),
    -- HMAC key for the JSON ping's signature; shown once when set.
    secret       TEXT NOT NULL,
    enabled      BOOLEAN NOT NULL DEFAULT true,
    failures     INTEGER NOT NULL DEFAULT 0,
    last_sent_at TIMESTAMPTZ,
    last_ok_at   TIMESTAMPTZ,
    last_error   TEXT,
    disabled_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
