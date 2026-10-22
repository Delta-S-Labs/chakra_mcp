//! Who may decide on a consent request (spec §2).
//!
//! The publisher's list (`consent_approvers`), minus anyone who has since
//! left the agent's account. If the list is empty, or nobody on it is still
//! a member, the account's owners and admins decide, so there is always
//! someone. Resolved when the decision is made.

use sqlx::PgPool;
use uuid::Uuid;

/// Whether `user_id` may approve or deny requests to a capability of the
/// account `account_id` whose publisher listed `approvers`.
pub async fn may_decide(
    db: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    approvers: &[Uuid],
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM account_memberships m
             WHERE m.account_id = $2 AND m.user_id = $1
               AND (m.user_id = ANY($3)
                    OR (m.role IN ('owner', 'admin')
                        AND NOT EXISTS (
                            SELECT 1 FROM account_memberships listed
                             WHERE listed.account_id = $2 AND listed.user_id = ANY($3))))
        ) AS "may_decide!"
        "#,
        user_id,
        account_id,
        approvers,
    )
    .fetch_one(db)
    .await
}

/// The people who may decide, for the approval page: id and display name.
pub async fn effective_approvers(
    db: &PgPool,
    account_id: Uuid,
    approvers: &[Uuid],
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.display_name
          FROM account_memberships m
          JOIN users u ON u.id = m.user_id
         WHERE m.account_id = $1
           AND (m.user_id = ANY($2)
                OR (m.role IN ('owner', 'admin')
                    AND NOT EXISTS (
                        SELECT 1 FROM account_memberships listed
                         WHERE listed.account_id = $1 AND listed.user_id = ANY($2))))
         ORDER BY u.display_name
        "#,
        account_id,
        approvers,
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.display_name)).collect())
}
