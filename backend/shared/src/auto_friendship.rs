//! Helpers for the per-org auto-friendship policy (introduced in PR-H).
//!
//! When an organization account has `auto_friendship_enabled = true`,
//! every pair of agents owned by accounts that *share* membership in
//! that org becomes instantly-accepted friends. "Sharing membership"
//! means: there exists an organization-type account `O` such that both
//! the caller and a member of the agent's owning account are members
//! of `O`. A caller who is themselves a member of the owning account
//! trivially satisfies this.
//!
//! Three triggers fire `backfill_for_org`:
//!   * settings PUT — when the operator flips the flag false → true
//!   * agent create — for every auto-friendship-enabled org the new
//!     agent's owning account is in scope of
//!   * invite accept — for the org the user just joined (if it has
//!     the flag on)
//!
//! All callers pass a `&PgPool` and run inside their own transaction
//! semantics — the helper itself just performs one bulk
//! `INSERT … ON CONFLICT DO NOTHING`-equivalent statement, so calling
//! it outside a transaction is safe too. Returns the number of new
//! rows created so callers can log or surface a count.

use sqlx::PgExecutor;
use uuid::Uuid;

/// Create accepted-status friendship rows for every cross-account
/// pair of agents in `org_account_id`'s scope that doesn't already
/// have an active friendship in either direction.
///
/// Scope = agents whose owning account either *is* the org or has at
/// least one member who is also a member of the org. Same-account
/// pairs are skipped (agents in the same account don't need a
/// friendship to invoke each other). Pair direction is canonicalised
/// to `(LEAST, GREATEST)` so we never insert both `(a, b)` and
/// `(b, a)`; the manual `NOT EXISTS` check considers BOTH directions
/// so we don't duplicate user-proposed friendships.
///
/// Idempotent: re-running is a no-op once the scope is friend-saturated.
pub async fn backfill_for_org<'e, E>(executor: E, org_account_id: Uuid) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let res = sqlx::query!(
        r#"
        WITH scope AS (
            SELECT DISTINCT acc.id AS account_id
            FROM accounts acc
            WHERE acc.tombstoned_at IS NULL
              AND (
                  acc.id = $1
                  OR EXISTS (
                      SELECT 1 FROM account_memberships am
                      JOIN account_memberships orgm
                          ON orgm.user_id = am.user_id
                         AND orgm.account_id = $1
                      WHERE am.account_id = acc.id
                  )
              )
        ),
        scope_agents AS (
            SELECT a.id, a.account_id
            FROM agents a
            JOIN scope s ON s.account_id = a.account_id
            WHERE a.tombstoned_at IS NULL
        )
        INSERT INTO friendships
            (id, proposer_agent_id, target_agent_id, status, decided_at, provenance)
        SELECT
            gen_random_uuid(),
            LEAST(a1.id, a2.id),
            GREATEST(a1.id, a2.id),
            'accepted',
            now(),
            jsonb_build_object('source', 'auto_friendship', 'org_account_id', $1)
        FROM scope_agents a1
        JOIN scope_agents a2
            ON a1.id < a2.id
           AND a1.account_id <> a2.account_id
        WHERE NOT EXISTS (
            SELECT 1 FROM friendships f
            WHERE f.status IN ('proposed', 'accepted')
              AND (
                  (f.proposer_agent_id = LEAST(a1.id, a2.id) AND f.target_agent_id = GREATEST(a1.id, a2.id))
                  OR
                  (f.proposer_agent_id = GREATEST(a1.id, a2.id) AND f.target_agent_id = LEAST(a1.id, a2.id))
              )
        )
        "#,
        org_account_id,
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}
