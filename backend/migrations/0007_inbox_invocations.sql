-- Phase 1.5 (revised) — invocations move from webhook-push to inbox-pull.
--
-- Why: nearly every real personal agent runs on a laptop behind NAT and
-- can't expose a public webhook. Pull-based delivery means the granter
-- side polls for work and posts results back, no inbound HTTP needed.
--
-- New status values:
--   pending     — enqueued, waiting for the granter side to pick it up.
--   in_progress — claimed via /v1/inbox, granter is working on it.
-- Existing terminal states (succeeded, failed, timeout, rejected) are
-- unchanged.
--
-- Two new bookkeeping columns:
--   claimed_at         — when the granter side pulled it from the inbox
--   claimed_by_user_id — which user did the pulling
--
-- Drop and re-create the status CHECK to expand the allowed set. The
-- existing default ('rejected' is set in app code) is preserved.

ALTER TABLE relay_invocations
    DROP CONSTRAINT IF EXISTS relay_invocations_status_check;

ALTER TABLE relay_invocations
    ADD CONSTRAINT relay_invocations_status_check
    CHECK (status IN ('pending', 'in_progress', 'rejected', 'succeeded', 'failed', 'timeout'));

ALTER TABLE relay_invocations
    ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ;
ALTER TABLE relay_invocations
    ADD COLUMN IF NOT EXISTS claimed_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL;

-- Inbox claim queries: "give me the oldest pending rows for these agents".
-- Filtered partial index so the planner can serve the inbox without
-- scanning the whole audit log once it grows.
CREATE INDEX IF NOT EXISTS idx_invocations_inbox
    ON relay_invocations (granter_agent_id, created_at)
    WHERE status = 'pending';
