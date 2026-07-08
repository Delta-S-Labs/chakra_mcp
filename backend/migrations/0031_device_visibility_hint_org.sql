-- Widen the pairing visibility hint to admit 'org', matching the tier that
-- migration 0021 added to agents.visibility / agent_capabilities.visibility.
--
-- The CHECK from 0030 predated the org tier reaching the pairing flow, so a
-- client could hint 'private' or 'network' but never 'org' — even though the
-- agent it pairs can hold 'org' just fine.
--
-- Postgres auto-names a column CHECK `<table>_<column>_check`, which is
-- exactly the name 0030's inline `ADD COLUMN ... CHECK (...)` produced
-- (verified against the live schema), so this DROP matches it. Same approach
-- migration 0021 used to widen `agents_visibility_check`.
--
-- NULL still means "no hint — fall back to the default".
ALTER TABLE oauth_device_codes
    DROP CONSTRAINT IF EXISTS oauth_device_codes_agent_visibility_hint_check;

ALTER TABLE oauth_device_codes
    ADD CONSTRAINT oauth_device_codes_agent_visibility_hint_check
    CHECK (agent_visibility_hint IS NULL
           OR agent_visibility_hint IN ('private', 'org', 'network'));
