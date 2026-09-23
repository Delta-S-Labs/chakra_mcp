-- Optional free-text purpose on grants: *why* the granter is giving the
-- grantee access to this capability ("book dinner reservations for the
-- team offsite", "read-only lookups for the weekly report").
--
-- Two readers:
--   * humans + the grantee agent — shown on grant cards and carried in
--     the inbox `grant` context so the serving side knows the intent;
--   * the System One compliance check (SYSTEM_ONE_CHECKS, relay
--     `compliance` module) — when set, each invocation's input is judged
--     against it in addition to the capability's own description.
--
-- Nullable, no backfill: existing grants simply have no stated purpose
-- and the compliance check skips the purpose question for them. Length
-- is capped here as well as in the handler so a direct INSERT can't
-- smuggle an unbounded blob into every compliance request.

ALTER TABLE grants
    ADD COLUMN IF NOT EXISTS purpose TEXT
        CHECK (purpose IS NULL OR char_length(purpose) BETWEEN 1 AND 500);
