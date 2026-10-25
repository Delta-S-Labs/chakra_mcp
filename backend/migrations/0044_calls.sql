-- Calls, C1: the tables (spec §13).
-- Design: docs/superpowers/specs/2026-10-25-calls-design.md.
--
-- All new, so nothing waits on a lock. Agents are referenced with
-- ON DELETE SET NULL, as relay_invocations does: deleting an org cascades to
-- its tombstoned agents, and a call must not stop that. Calls are never
-- deleted; when their TTL runs out, their content is (a stub stays).

-- ─── calls ───────────────────────────────────────────────
CREATE TABLE calls (
    id                 UUID PRIMARY KEY,
    caller_agent_id    UUID REFERENCES agents(id) ON DELETE SET NULL,
    callee_agent_id    UUID REFERENCES agents(id) ON DELETE SET NULL,
    -- The accounts at invite, for history only: visibility and billing
    -- follow each agent's current account.
    caller_account_id  UUID NOT NULL,
    callee_account_id  UUID NOT NULL,
    started_by_user_id UUID,
    memory             TEXT NOT NULL CHECK (memory IN ('call', 'own')),
    -- What call memory can do on this relay, fixed at invite.
    memory_features    JSONB NOT NULL,
    ttl_secs           INTEGER NOT NULL CHECK (ttl_secs >= 0),
    idle_secs          INTEGER NOT NULL CHECK (idle_secs BETWEEN 3600 AND 604800),
    topic              TEXT,
    decline_reason     TEXT,
    state              TEXT NOT NULL
        CHECK (state IN ('ringing', 'active', 'declined', 'missed', 'ended')),
    ended_by           TEXT CHECK (ended_by IN ('caller', 'callee', 'owner', 'idle', 'gone')),
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    answered_at        TIMESTAMPTZ,
    ended_at           TIMESTAMPTZ,
    last_message_at    TIMESTAMPTZ,
    purge_at           TIMESTAMPTZ,
    purged_at          TIMESTAMPTZ,
    -- The last transcript `seq` handed out; entries take the next one with
    -- UPDATE … RETURNING, which orders them per call.
    last_seq           INTEGER NOT NULL DEFAULT 0,
    message_count      INTEGER NOT NULL DEFAULT 0,
    memory_cost_mc     BIGINT NOT NULL DEFAULT 0,
    CHECK (caller_agent_id IS NULL OR caller_agent_id IS DISTINCT FROM callee_agent_id)
);

CREATE INDEX calls_caller_state ON calls (caller_agent_id, state);
CREATE INDEX calls_callee_state ON calls (callee_agent_id, state);
-- At most one ringing call per caller and callee.
CREATE UNIQUE INDEX calls_ringing_pair ON calls (caller_agent_id, callee_agent_id)
    WHERE state = 'ringing';
CREATE INDEX calls_ringing ON calls (created_at) WHERE state = 'ringing';
CREATE INDEX calls_active ON calls (last_message_at) WHERE state = 'active';
CREATE INDEX calls_purge_due ON calls (purge_at)
    WHERE purged_at IS NULL AND purge_at IS NOT NULL;

-- ─── call_entries: the transcript ────────────────────────
CREATE TABLE call_entries (
    id            UUID PRIMARY KEY,
    call_id       UUID NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('invite', 'message', 'accepted', 'declined',
                                                'ended', 'missed', 'invocation')),
    from_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    -- Content, deleted when the call's TTL runs out.
    text          TEXT,
    data          JSONB,
    -- For `invocation` entries; status and previews are read by join.
    invocation_id UUID,
    blocked       BOOLEAN NOT NULL DEFAULT false,
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    -- Message safety's findings (never a removed value).
    safety        JSONB,
    -- Call memory (C3): `pending` until Jev has tagged the message.
    tag_state     TEXT CHECK (tag_state IN ('pending', 'done', 'error')),
    tags          JSONB,
    tagged_at     TIMESTAMPTZ,
    -- 'simple' has no stemming or stop words, so it works in any language.
    search        TSVECTOR GENERATED ALWAYS AS
                      (to_tsvector('simple'::regconfig, coalesce(text, ''))) STORED,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (call_id, seq)
);

CREATE INDEX call_entries_untagged ON call_entries (call_id, created_at)
    WHERE tag_state = 'pending';
CREATE INDEX call_entries_search ON call_entries USING GIN (search);
CREATE INDEX call_entries_invocation ON call_entries (invocation_id)
    WHERE invocation_id IS NOT NULL;

-- ─── call_deliveries: who still has to pull an entry ─────
CREATE TABLE call_deliveries (
    entry_id     UUID NOT NULL REFERENCES call_entries(id) ON DELETE CASCADE,
    -- A deleted agent has nothing left to pull.
    agent_id     UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    PRIMARY KEY (entry_id, agent_id)
);

CREATE INDEX call_deliveries_pending ON call_deliveries (agent_id, created_at)
    WHERE delivered_at IS NULL;

-- ─── Billing: messages and memory work ───────────────────
-- Like credit_charge_queue, but each row carries its own cost: a message's
-- is fixed, memory's is metered per request. Rows are inserted with the
-- thing they pay for and drained by the credits worker into call_charges,
-- whose key makes double charging impossible. No FKs: billing history
-- outlives what it paid for.
CREATE TABLE call_charge_queue (
    id         UUID PRIMARY KEY,
    account_id UUID NOT NULL,
    call_id    UUID NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('message', 'memory')),
    cost_mc    BIGINT NOT NULL CHECK (cost_mc >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
) WITH (autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);

CREATE TABLE call_charges (
    id         UUID PRIMARY KEY,
    account_id UUID NOT NULL,
    call_id    UUID NOT NULL,
    kind       TEXT NOT NULL,
    cost_mc    BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    charged_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX call_charges_account ON call_charges (account_id, charged_at);

-- ─── Search ──────────────────────────────────────────────
-- `GET /v1/calls/{id}/messages?q=` and recall (spec §8.4): the words of `q`
-- as 'simple' lexemes, dropping those of one or two characters, ORed, so
-- any of them matches (an AND of every word in a question would match
-- almost nothing). NULL, which matches nothing, when no word is left.
CREATE FUNCTION calls_or_query(q TEXT) RETURNS tsquery
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT to_tsquery('simple'::regconfig, string_agg(quote_literal(t.lexeme), ' | '))
      FROM unnest(to_tsvector('simple'::regconfig, q)) AS t
     WHERE length(t.lexeme) >= 3
$$;
