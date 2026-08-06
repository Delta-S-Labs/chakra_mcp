//! Per-account monthly invocation quota, tracked durably in
//! `usage_counters` (migration 0032).
//!
//! `current_month` reads the caller account's count for the current UTC
//! calendar month; `increment` bumps it. The increment is designed to run
//! inside the same transaction that writes the `relay_invocations` row, so
//! the counter never drifts from the ledger (see PR 3 wiring).

use uuid::Uuid;

/// Invocations recorded for `account_id` in the current UTC calendar month.
/// Returns 0 when no counter row exists yet.
pub async fn current_month<'e, E>(db: E, account_id: Uuid) -> Result<i64, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    let row = sqlx::query!(
        r#"
        SELECT invocations
          FROM usage_counters
         WHERE account_id = $1
           AND period_start = date_trunc('month', now() AT TIME ZONE 'UTC')::date
        "#,
        account_id,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.invocations).unwrap_or(0))
}

/// Increment the current-month counter for `account_id` by one, creating the
/// row on first use. Intended to be called with a `&mut *tx` inside the
/// `relay_invocations` write transaction.
pub async fn increment<'e, E>(db: E, account_id: Uuid) -> Result<(), sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        INSERT INTO usage_counters (account_id, period_start, invocations)
        VALUES ($1, date_trunc('month', now() AT TIME ZONE 'UTC')::date, 1)
        ON CONFLICT (account_id, period_start)
        DO UPDATE SET invocations = usage_counters.invocations + 1
        "#,
        account_id,
    )
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    /// Seed a user + account (defaulting to the free plan via migration 0032)
    /// and return the account id.
    async fn seed_account(pool: &PgPool) -> Uuid {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'quota-test')",
            user_id,
            format!("quota-{}@t.local", user_id.simple()),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'quota-test', 'individual', $3)"#,
            account_id,
            format!("quota-{}", &account_id.simple().to_string()[..12]),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        account_id
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn current_month_is_zero_before_any_increment(pool: PgPool) {
        let acct = seed_account(&pool).await;
        assert_eq!(current_month(&pool, acct).await.unwrap(), 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn increment_accumulates_within_the_month(pool: PgPool) {
        let acct = seed_account(&pool).await;
        for _ in 0..5 {
            increment(&pool, acct).await.unwrap();
        }
        assert_eq!(current_month(&pool, acct).await.unwrap(), 5);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn increment_is_per_account(pool: PgPool) {
        let a = seed_account(&pool).await;
        let b = seed_account(&pool).await;
        increment(&pool, a).await.unwrap();
        increment(&pool, a).await.unwrap();
        increment(&pool, b).await.unwrap();
        assert_eq!(current_month(&pool, a).await.unwrap(), 2);
        assert_eq!(current_month(&pool, b).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn increment_rolls_back_with_its_transaction(pool: PgPool) {
        let acct = seed_account(&pool).await;
        let mut tx = pool.begin().await.unwrap();
        increment(&mut *tx, acct).await.unwrap();
        tx.rollback().await.unwrap();
        // The increment vanished with the rolled-back transaction.
        assert_eq!(current_month(&pool, acct).await.unwrap(), 0);
    }
}
