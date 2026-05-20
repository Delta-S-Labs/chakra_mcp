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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// One personal account + one user, returns their ids.
    async fn seed_user(pool: &PgPool) -> (Uuid, Uuid) {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'T', 'x')"#,
            user_id,
            format!("{user_id}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Personal', 'individual', $3)"#,
            account_id,
            format!("personal-{account_id}"),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        (user_id, account_id)
    }

    /// Org account with `user_id` as owner. Returns the org's account_id.
    async fn seed_org(pool: &PgPool, owner_user_id: Uuid, slug: &str) -> Uuid {
        let org_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id, auto_friendship_enabled)
               VALUES ($1, $2, $3, 'organization', $4, true)"#,
            org_id,
            slug,
            slug,
            owner_user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            org_id,
            owner_user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        org_id
    }

    /// Make `user_id` a member of `account_id`. Used to put two users in
    /// the same org so their personal accounts share scope.
    async fn add_member(pool: &PgPool, account_id: Uuid, user_id: Uuid) {
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'member')"#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Plain agent under `account_id`. Returns the new agent's id.
    async fn seed_agent(pool: &PgPool, account_id: Uuid, slug: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents
                 (id, account_id, slug, display_name, description, visibility, mode)
               VALUES ($1, $2, $3, $4, '', 'private', 'pull')"#,
            id,
            account_id,
            slug,
            slug,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn count_friendships(pool: &PgPool) -> i64 {
        sqlx::query_scalar!(r#"SELECT COUNT(*) as "n!" FROM friendships"#)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn friendship_pair_exists(pool: &PgPool, a: Uuid, b: Uuid, status: &str) -> bool {
        sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM friendships
                WHERE status = $3
                  AND (
                      (proposer_agent_id = $1 AND target_agent_id = $2)
                   OR (proposer_agent_id = $2 AND target_agent_id = $1)
                  )
            ) as "e!"
            "#,
            a,
            b,
            status,
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The happy-path: two users sharing an org → their personal-account
    /// agents auto-friend each other plus the org's own agent.
    #[sqlx::test(migrations = "../migrations")]
    async fn backfills_cross_account_pairs_in_scope(pool: PgPool) {
        let (alice_user, alice_acct) = seed_user(&pool).await;
        let (bob_user, bob_acct) = seed_user(&pool).await;
        let org = seed_org(&pool, alice_user, "shared-org").await;
        add_member(&pool, org, bob_user).await;

        let alice_agent = seed_agent(&pool, alice_acct, "alice-a").await;
        let bob_agent = seed_agent(&pool, bob_acct, "bob-a").await;
        let org_agent = seed_agent(&pool, org, "org-a").await;

        let n = backfill_for_org(&pool, org).await.unwrap();
        // 3 unordered pairs: (alice,bob), (alice,org), (bob,org). All
        // cross-account so all qualify.
        assert_eq!(n, 3, "expected 3 friendships; got {n}");
        assert!(friendship_pair_exists(&pool, alice_agent, bob_agent, "accepted").await);
        assert!(friendship_pair_exists(&pool, alice_agent, org_agent, "accepted").await);
        assert!(friendship_pair_exists(&pool, bob_agent, org_agent, "accepted").await);
    }

    /// Re-running backfill on a saturated scope inserts zero new rows.
    #[sqlx::test(migrations = "../migrations")]
    async fn idempotent_on_saturated_scope(pool: PgPool) {
        let (alice_user, alice_acct) = seed_user(&pool).await;
        let (bob_user, bob_acct) = seed_user(&pool).await;
        let org = seed_org(&pool, alice_user, "sat-org").await;
        add_member(&pool, org, bob_user).await;
        seed_agent(&pool, alice_acct, "a").await;
        seed_agent(&pool, bob_acct, "b").await;

        let first = backfill_for_org(&pool, org).await.unwrap();
        let second = backfill_for_org(&pool, org).await.unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 0, "rerun should be a no-op");
        assert_eq!(count_friendships(&pool).await, 1);
    }

    /// Two agents owned by the SAME account never get friended even when
    /// that account is in scope. Friendships are inter-account.
    #[sqlx::test(migrations = "../migrations")]
    async fn skips_same_account_pairs(pool: PgPool) {
        let (owner_user, _personal) = seed_user(&pool).await;
        let org = seed_org(&pool, owner_user, "same-org").await;
        // Two org-owned agents; no other account in scope.
        seed_agent(&pool, org, "org-a").await;
        seed_agent(&pool, org, "org-b").await;

        let n = backfill_for_org(&pool, org).await.unwrap();
        assert_eq!(n, 0, "same-account pairs must not auto-friend");
        assert_eq!(count_friendships(&pool).await, 0);
    }

    /// Pre-existing active friendship in either direction blocks the
    /// auto-row — both (a,b) and (b,a) variants get checked.
    #[sqlx::test(migrations = "../migrations")]
    async fn respects_existing_proposed_in_either_direction(pool: PgPool) {
        let (alice_user, alice_acct) = seed_user(&pool).await;
        let (bob_user, bob_acct) = seed_user(&pool).await;
        let org = seed_org(&pool, alice_user, "blocked-org").await;
        add_member(&pool, org, bob_user).await;
        let a = seed_agent(&pool, alice_acct, "alice-a").await;
        let b = seed_agent(&pool, bob_acct, "bob-a").await;

        // Manually propose (a, b) — proposed status, alice's side.
        sqlx::query!(
            r#"INSERT INTO friendships
                 (id, proposer_agent_id, target_agent_id, status, proposer_user_id)
               VALUES ($1, $2, $3, 'proposed', $4)"#,
            Uuid::now_v7(),
            a,
            b,
            alice_user,
        )
        .execute(&pool)
        .await
        .unwrap();

        let n = backfill_for_org(&pool, org).await.unwrap();
        assert_eq!(n, 0, "must not duplicate an existing proposed row");
        assert_eq!(count_friendships(&pool).await, 1);

        // Now flip to the OTHER direction: clear and try (b, a) proposed.
        sqlx::query!(r#"DELETE FROM friendships"#)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query!(
            r#"INSERT INTO friendships
                 (id, proposer_agent_id, target_agent_id, status, proposer_user_id)
               VALUES ($1, $2, $3, 'proposed', $4)"#,
            Uuid::now_v7(),
            b,
            a,
            bob_user,
        )
        .execute(&pool)
        .await
        .unwrap();

        let n = backfill_for_org(&pool, org).await.unwrap();
        assert_eq!(n, 0, "must not duplicate a reverse-direction proposed row");
        assert_eq!(count_friendships(&pool).await, 1);
    }

    /// A `rejected` row in the table is NOT considered active, so the
    /// backfill is free to create a fresh `accepted` row for the pair.
    #[sqlx::test(migrations = "../migrations")]
    async fn ignores_inactive_rejected_rows(pool: PgPool) {
        let (alice_user, alice_acct) = seed_user(&pool).await;
        let (bob_user, bob_acct) = seed_user(&pool).await;
        let org = seed_org(&pool, alice_user, "rej-org").await;
        add_member(&pool, org, bob_user).await;
        let a = seed_agent(&pool, alice_acct, "alice-a").await;
        let b = seed_agent(&pool, bob_acct, "bob-a").await;

        // Past rejection. Should not block a fresh auto-friendship.
        sqlx::query!(
            r#"INSERT INTO friendships
                 (id, proposer_agent_id, target_agent_id, status, proposer_user_id,
                  decided_by_user_id, decided_at)
               VALUES ($1, $2, $3, 'rejected', $4, $5, now())"#,
            Uuid::now_v7(),
            a,
            b,
            alice_user,
            bob_user,
        )
        .execute(&pool)
        .await
        .unwrap();

        let n = backfill_for_org(&pool, org).await.unwrap();
        assert_eq!(n, 1, "rejected is inactive; backfill should add one");
        assert!(friendship_pair_exists(&pool, a, b, "accepted").await);
    }

    /// Tombstoned agents drop out of the scope completely.
    #[sqlx::test(migrations = "../migrations")]
    async fn ignores_tombstoned_agents(pool: PgPool) {
        let (alice_user, alice_acct) = seed_user(&pool).await;
        let (bob_user, bob_acct) = seed_user(&pool).await;
        let org = seed_org(&pool, alice_user, "tomb-org").await;
        add_member(&pool, org, bob_user).await;
        seed_agent(&pool, alice_acct, "alice-live").await;
        let bob_dead = seed_agent(&pool, bob_acct, "bob-dead").await;
        sqlx::query!(
            r#"UPDATE agents SET tombstoned_at = now() WHERE id = $1"#,
            bob_dead,
        )
        .execute(&pool)
        .await
        .unwrap();

        let n = backfill_for_org(&pool, org).await.unwrap();
        // Alice-live alone in scope — nobody to friend with.
        assert_eq!(n, 0);
    }

    /// Called with an org that has nobody in scope but itself: no agents
    /// → no work. (Triggers the early-exit cheaply.)
    #[sqlx::test(migrations = "../migrations")]
    async fn empty_scope_returns_zero(pool: PgPool) {
        let (owner_user, _personal) = seed_user(&pool).await;
        let org = seed_org(&pool, owner_user, "empty-org").await;
        let n = backfill_for_org(&pool, org).await.unwrap();
        assert_eq!(n, 0);
        assert_eq!(count_friendships(&pool).await, 0);
    }
}
