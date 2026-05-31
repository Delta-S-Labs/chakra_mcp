-- Edge 2 from the post-SP2 trust-context audit: freeze the
-- friendship + grant state at queue time on `relay_invocations`
-- so audit-log endpoints can return what was *actually* true when
-- the call happened, not what's true now.
--
-- Before this column: `GET /v1/invocations` / `GET /v1/invocations/{id}`
-- returned `friendship_context: null` and `grant_context: null` —
-- the relay refused to compute them at audit-log read time because
-- the live state might have drifted since the row was queued (the
-- friendship could have been countered, the grant revoked, etc.).
--
-- With this column: the relay snapshots both rows at INSERT time
-- (inside POST /v1/invoke, when it just verified them). Audit-log
-- endpoints deserialize the snapshot back into the same typed
-- contexts that `inbox.pull` returns — so an auditor sees what the
-- granter saw at delivery time, with the same shape, no separate
-- field to learn.
--
-- For public-invoke rows (`grant_id IS NULL`) the snapshot is NULL
-- — there's no friendship/grant to freeze; the relay's per-invoker
-- quota gate is the trust assertion and that's already on the row
-- via `invoked_by_user_id`/`api_key_id`.
--
-- For legacy rows pre-this-migration the snapshot is also NULL —
-- those continue to render with null contexts on audit-log
-- endpoints, same as before. No backfill (we can't reconstruct
-- the state-as-of-queue-time after the fact without risking
-- post-revoke state leakage).

ALTER TABLE relay_invocations
    ADD COLUMN trust_snapshot JSONB;

COMMENT ON COLUMN relay_invocations.trust_snapshot IS
    'Friendship + grant state captured at queue time (POST /v1/invoke). '
    'Shape: {"friendship": {<FriendshipContext>}, "grant": {<GrantContext>}}. '
    'NULL on public-invoke rows and on rows from before migration 0024. '
    'Returned verbatim by audit-log endpoints so they show state-at-the-time '
    'instead of state-now.';
