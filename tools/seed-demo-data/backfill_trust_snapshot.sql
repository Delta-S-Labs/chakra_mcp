-- Backfill trust_snapshot on demo invocations (one-time, demo data only).
--
-- The YC demo invocations were seeded through the live invoke path on
-- 2026-05-30, but migration 0024 + the snapshot-writing code only
-- deployed 2026-05-31 (PR #146). So every demo row is grant-path
-- (grant_id set) but has trust_snapshot = NULL — which makes the new
-- audit "Trust context" UI (PR #149) show "no snapshot on this row"
-- for the entire demo. This synthesises the snapshot from the current
-- grant + accepted friendship + capability so the demo actually
-- exercises the feature.
--
-- Shape matches exactly what POST /v1/invoke now writes (invoke.rs):
--   { "grant": {<GrantContext>}, "friendship": {<FriendshipContext>} }
-- so the relay's contexts_from_snapshot() deserialises it the same way
-- as a natively-written row.
--
-- This is NOT the production migration's behaviour — 0024 deliberately
-- does NOT backfill (post-revoke state leakage). It's safe here only
-- because this is a controlled demo dataset whose grants/friendships
-- haven't drifted since the calls were made. Idempotent: only touches
-- grant-path rows that are still missing a snapshot.

BEGIN;

UPDATE relay_invocations i
SET trust_snapshot = jsonb_build_object(
    'grant', jsonb_build_object(
        'id', g.id,
        'status', g.status,
        'granter_agent_id', g.granter_agent_id,
        'grantee_agent_id', g.grantee_agent_id,
        'capability_id', g.capability_id,
        'capability_name', c.name,
        'capability_visibility', c.visibility,
        'granted_at', g.granted_at,
        'expires_at', g.expires_at
    ),
    'friendship', (
        SELECT jsonb_build_object(
            'id', f.id,
            'status', f.status,
            'proposer_agent_id', f.proposer_agent_id,
            'target_agent_id', f.target_agent_id,
            'proposer_message', f.proposer_message,
            'response_message', f.response_message,
            'decided_at', f.decided_at
        )
        FROM friendships f
        WHERE f.status = 'accepted'
          AND (
              (f.proposer_agent_id = g.granter_agent_id AND f.target_agent_id = g.grantee_agent_id)
           OR (f.proposer_agent_id = g.grantee_agent_id AND f.target_agent_id = g.granter_agent_id)
          )
        ORDER BY f.decided_at NULLS LAST
        LIMIT 1
    )
)
FROM grants g
JOIN agent_capabilities c ON c.id = g.capability_id
WHERE i.grant_id = g.id
  AND i.grant_id IS NOT NULL
  AND i.trust_snapshot IS NULL;

-- Report what we touched.
SELECT
    count(*) FILTER (WHERE trust_snapshot IS NOT NULL) AS with_snapshot,
    count(*) FILTER (WHERE trust_snapshot IS NULL)     AS without_snapshot,
    count(*) AS total
FROM relay_invocations
WHERE grant_id IS NOT NULL;

COMMIT;
