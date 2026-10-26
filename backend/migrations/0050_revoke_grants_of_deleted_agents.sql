-- Deleting an agent didn't revoke its grants, so a deleted agent could still
-- call, and be called, through them. Deletes revoke them now; this revokes the
-- active grants of agents (or accounts) already deleted, and rejects requests
-- still queued on them. Only those rows are touched.
WITH revoked AS (
    UPDATE grants g
       SET status = 'revoked',
           revoked_at = now(),
           revoke_reason = 'agent deleted'
     WHERE g.status = 'active'
       AND EXISTS (
           SELECT 1
             FROM agents a
             JOIN accounts acc ON acc.id = a.account_id
            WHERE a.id IN (g.granter_agent_id, g.grantee_agent_id)
              AND (a.tombstoned_at IS NOT NULL OR acc.tombstoned_at IS NOT NULL))
    RETURNING g.id
)
UPDATE relay_invocations
   SET status = 'rejected',
       error_message = COALESCE(error_message, 'agent deleted while queued')
 WHERE status = 'pending'
   AND grant_id IN (SELECT id FROM revoked);
