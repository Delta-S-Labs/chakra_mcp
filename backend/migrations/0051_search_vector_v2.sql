-- MCP and discovery D2: ranked search (spec §5.1, §8).
-- Design: docs/superpowers/specs/2026-10-27-mcp-discovery-design.md.
--
-- - The slug joins the vector at weight A, beside the display name.
-- - Each field gets a `simple` and an `english` vector at the same weight,
--   so "booking" finds "book" and names still match exactly.
-- - Capability text comes only from network-visible capabilities: private
--   and org-visible ones stop leaking into search.
-- - `discovery_query(q)` turns free text into an OR of its words.
--
-- Locking: `agents` is read on every invocation, so this never takes a
-- lock that blocks readers. `CREATE OR REPLACE TRIGGER` needs only SHARE
-- ROW EXCLUSIVE (Postgres 14+; 16 is the documented minimum), which blocks
-- writes to `agents` but not reads. It's taken as 0036 takes its locks: try
-- for 100 ms, and on a timeout back off and retry, so a waiting request
-- never queues writers for long. It's held to the end of the migration,
-- through the re-index: about 2 s at 5,000 agents with 3 capabilities each
-- (measured locally), a few hundred ms at today's size. The re-index moves
-- every agent's `updated_at` through its trigger.

-- The vector for one agent. Weights: A = display name and slug; B = account
-- name and tags; C = description and capability text.
CREATE OR REPLACE FUNCTION agents_search_vec_compose(
    slug TEXT,
    display_name TEXT,
    description TEXT,
    account_display_name TEXT,
    tags TEXT[],
    cap_text TEXT
) RETURNS tsvector AS $$
    SELECT
        setweight(to_tsvector('simple',  coalesce(display_name, '') || ' ' || coalesce(slug, '')), 'A')
     || setweight(to_tsvector('english', coalesce(display_name, '') || ' ' || coalesce(slug, '')), 'A')
     || setweight(to_tsvector('simple',  coalesce(account_display_name, '') || ' '
                                         || array_to_string(coalesce(tags, '{}'::text[]), ' ')), 'B')
     || setweight(to_tsvector('english', coalesce(account_display_name, '') || ' '
                                         || array_to_string(coalesce(tags, '{}'::text[]), ' ')), 'B')
     || setweight(to_tsvector('simple',  coalesce(description, '') || ' ' || coalesce(cap_text, '')), 'C')
     || setweight(to_tsvector('english', coalesce(description, '') || ' ' || coalesce(cap_text, '')), 'C')
$$ LANGUAGE SQL IMMUTABLE;

-- An agent's network-visible capability text: names and descriptions.
CREATE OR REPLACE FUNCTION agents_search_cap_text(agent UUID) RETURNS TEXT AS $$
    SELECT coalesce(string_agg(coalesce(name, '') || ' ' || coalesce(description, ''), ' '), '')
      FROM agent_capabilities
     WHERE agent_id = agent AND visibility = 'network'
$$ LANGUAGE SQL STABLE;

CREATE OR REPLACE FUNCTION agents_refresh_search_vec() RETURNS trigger AS $$
BEGIN
    NEW.search_vec := agents_search_vec_compose(
        NEW.slug,
        NEW.display_name,
        NEW.description,
        (SELECT display_name FROM accounts WHERE id = NEW.account_id),
        NEW.tags,
        agents_search_cap_text(NEW.id)
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Free text to a tsquery: an OR over its words, each as its `simple`
-- lexeme and its English stems. Words of three or more characters are
-- prefix-matched ("calend" finds "calendar"); shorter ones ("ai", "hr")
-- match whole. Lexemes are built from alphanumeric runs and quoted, so no
-- input reaches tsquery syntax. NULL when no word survives; the first 16
-- words count.
CREATE OR REPLACE FUNCTION discovery_query(q TEXT) RETURNS tsquery AS $$
DECLARE
    word   TEXT;
    stem   TEXT;
    suffix TEXT;
    parts  TEXT[] := '{}';
    n      INT := 0;
BEGIN
    FOREACH word IN ARRAY regexp_split_to_array(lower(coalesce(q, '')), '[^[:alnum:]]+') LOOP
        CONTINUE WHEN word = '';
        n := n + 1;
        EXIT WHEN n > 16;
        suffix := CASE WHEN char_length(word) >= 3 THEN ':*' ELSE '' END;
        parts := parts || ('''' || word || '''' || suffix);
        FOREACH stem IN ARRAY coalesce(ts_lexize('english_stem', word), '{}'::text[]) LOOP
            CONTINUE WHEN stem = word;
            parts := parts || ('''' || stem || '''' || suffix);
        END LOOP;
    END LOOP;
    IF cardinality(parts) = 0 THEN
        RETURN NULL;
    END IF;
    RETURN array_to_string(parts, ' | ')::tsquery;
END;
$$ LANGUAGE plpgsql STABLE;

-- The trigger, now also on slug changes.
DO $$
DECLARE
    attempt INT := 0;
BEGIN
    LOOP
        BEGIN
            SET LOCAL lock_timeout = '100ms';
            LOCK TABLE agents IN SHARE ROW EXCLUSIVE MODE;
            CREATE OR REPLACE TRIGGER agents_search_vec_update
                BEFORE INSERT OR UPDATE OF
                    display_name, description, account_id, tags, updated_at, slug
                ON agents
                FOR EACH ROW EXECUTE FUNCTION agents_refresh_search_vec();
            EXIT;
        EXCEPTION WHEN lock_not_available OR deadlock_detected THEN
            attempt := attempt + 1;
            IF attempt >= 50 THEN
                RAISE;
            END IF;
            PERFORM pg_sleep(LEAST(0.05 * attempt, 1.0));
        END;
    END LOOP;
END $$;

DROP FUNCTION IF EXISTS agents_search_vec_compose(TEXT, TEXT, TEXT, TEXT);

-- Re-index every agent.
UPDATE agents a
   SET search_vec = agents_search_vec_compose(
        a.slug,
        a.display_name,
        a.description,
        (SELECT display_name FROM accounts WHERE id = a.account_id),
        a.tags,
        agents_search_cap_text(a.id)
   );
