-- 0025: general usage + audit event tables.
--
-- Until now the only metered/audited surface was `relay_invocations`
-- (capability calls). This adds two general tables so EVERY relay request
-- can be metered for future billing, and every WRITE leaves an audit trail.
--
--   usage_events  — one row per API request (REST + per MCP tool), incl.
--                   read-only GETs/pulls. The billing substrate.
--   audit_events  — one row per write (create/update/delete/state change).
--                   Read-only pulls are intentionally NOT audited.
--
-- Ids are supplied by the app (Uuid::now_v7) like every other table here.

CREATE TABLE usage_events (
    id            UUID PRIMARY KEY,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Attribution (all nullable: an unauthenticated/discovery hit still meters).
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    account_id    UUID REFERENCES accounts(id) ON DELETE SET NULL,
    api_key_id    UUID REFERENCES api_keys(id) ON DELETE SET NULL,
    minted_jti    UUID,
    -- What happened.
    surface       TEXT NOT NULL,           -- 'rest' | 'mcp'
    action        TEXT NOT NULL,           -- e.g. 'GET /v1/friendships', 'mcp:pull_inbox'
    method        TEXT NOT NULL,           -- HTTP method, or 'MCP' for tool calls
    route         TEXT NOT NULL,           -- matched route template or tool name
    status_code   INTEGER NOT NULL,
    ok            BOOLEAN NOT NULL,
    CHECK (surface IN ('rest', 'mcp'))
);

-- Billing rollups are per-account and per-key over time windows.
CREATE INDEX usage_events_account_time_idx ON usage_events (account_id, created_at DESC);
CREATE INDEX usage_events_user_time_idx    ON usage_events (actor_user_id, created_at DESC);
CREATE INDEX usage_events_api_key_time_idx ON usage_events (api_key_id, created_at DESC)
    WHERE api_key_id IS NOT NULL;
CREATE INDEX usage_events_time_idx         ON usage_events (created_at DESC);


CREATE TABLE audit_events (
    id            UUID PRIMARY KEY,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    account_id    UUID REFERENCES accounts(id) ON DELETE SET NULL,
    api_key_id    UUID REFERENCES api_keys(id) ON DELETE SET NULL,
    minted_jti    UUID,
    -- Semantic action + the resource it touched.
    action        TEXT NOT NULL,           -- 'agent.create', 'friendship.accept', 'grant.revoke', …
    resource_type TEXT NOT NULL,           -- 'agent' | 'capability' | 'friendship' | 'grant' | 'review'
    resource_id   UUID,                    -- the primary resource
    target_id     UUID,                    -- optional secondary (e.g. peer agent on a friendship)
    summary       TEXT NOT NULL DEFAULT '',
    metadata      JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX audit_events_account_time_idx  ON audit_events (account_id, created_at DESC);
CREATE INDEX audit_events_user_time_idx     ON audit_events (actor_user_id, created_at DESC);
CREATE INDEX audit_events_resource_idx      ON audit_events (resource_type, resource_id);
