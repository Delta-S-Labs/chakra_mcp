-- Failed password sign-ins per email, for the sign-in limit: after 10
-- failures within 15 minutes, further attempts for that email are refused
-- until the window ends. Design:
-- docs/superpowers/specs/2026-10-01-self-hosting-phase3-design.md §6.5.
--
-- Additive: a new table nothing else reads. The key is the SHA-256 of the
-- lowercased email, so addresses typed by strangers aren't stored.

CREATE TABLE IF NOT EXISTS signin_failures (
    email_hash BYTEA PRIMARY KEY,
    failures INT NOT NULL,
    window_started_at TIMESTAMPTZ NOT NULL
);

-- Pruning finished windows (rows older than a day) uses this.
CREATE INDEX IF NOT EXISTS idx_signin_failures_window
    ON signin_failures (window_started_at);
