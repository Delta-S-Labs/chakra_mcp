//! The failed sign-in limit, shared by `POST /v1/auth/login` and the
//! built-in sign-in pages: after [`MAX_FAILURES`] failed passwords for one
//! email within [`WINDOW_SECS`], further attempts for that email are refused
//! until the window ends, even with the right password. A success clears the
//! count. Unknown emails count the same way, so the answer can't be used to
//! find accounts.
//!
//! Stored in Postgres (`signin_failures`, keyed by the SHA-256 of the
//! lowercased email), so it needs no Redis and holds across replicas. The
//! trade-off: someone who knows an email can block password sign-in for
//! that account, a window at a time; sessions, OAuth tokens and API keys
//! keep working, and the operator commands always do.

use sha2::{Digest, Sha256};
use sqlx::PgPool;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::accounts::normalize_email;

pub const MAX_FAILURES: i32 = 10;
pub const WINDOW_SECS: i64 = 15 * 60;

fn email_hash(email: &str) -> Vec<u8> {
    Sha256::digest(normalize_email(email).as_bytes()).to_vec()
}

/// `SigninRateLimited` while this email is over the limit.
pub async fn check(db: &PgPool, email: &str) -> ApiResult<()> {
    let retry_after = sqlx::query_scalar!(
        r#"
        SELECT GREATEST(1, EXTRACT(EPOCH FROM
                   window_started_at + make_interval(secs => $3) - now()))::bigint AS "retry_after!"
          FROM signin_failures
         WHERE email_hash = $1
           AND failures >= $2
           AND window_started_at > now() - make_interval(secs => $3)
        "#,
        email_hash(email),
        MAX_FAILURES,
        WINDOW_SECS as f64,
    )
    .fetch_optional(db)
    .await?;
    match retry_after {
        Some(secs) => Err(ApiError::SigninRateLimited {
            retry_after_secs: secs.max(1) as u64,
        }),
        None => Ok(()),
    }
}

/// Count a failed attempt, starting a new window when the last one ended,
/// and drop rows whose window ended more than a day ago.
pub async fn record_failure(db: &PgPool, email: &str) -> ApiResult<()> {
    sqlx::query!(
        r#"
        INSERT INTO signin_failures (email_hash, failures, window_started_at)
        VALUES ($1, 1, now())
        ON CONFLICT (email_hash) DO UPDATE SET
            failures = CASE
                WHEN signin_failures.window_started_at <= now() - make_interval(secs => $2)
                THEN 1 ELSE signin_failures.failures + 1 END,
            window_started_at = CASE
                WHEN signin_failures.window_started_at <= now() - make_interval(secs => $2)
                THEN now() ELSE signin_failures.window_started_at END
        "#,
        email_hash(email),
        WINDOW_SECS as f64,
    )
    .execute(db)
    .await?;
    sqlx::query!("DELETE FROM signin_failures WHERE window_started_at < now() - interval '1 day'")
        .execute(db)
        .await?;
    Ok(())
}

/// Forget the failures after a successful sign-in.
pub async fn clear(db: &PgPool, email: &str) -> ApiResult<()> {
    sqlx::query!(
        "DELETE FROM signin_failures WHERE email_hash = $1",
        email_hash(email)
    )
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "../migrations")]
    async fn ten_failures_lock_the_email_for_the_window(pool: PgPool) {
        for _ in 0..MAX_FAILURES - 1 {
            record_failure(&pool, "a@example.test").await.unwrap();
            check(&pool, "a@example.test").await.unwrap();
        }
        record_failure(&pool, " A@Example.TEST ").await.unwrap();
        match check(&pool, "a@example.test").await {
            Err(ApiError::SigninRateLimited { retry_after_secs }) => {
                assert!((1..=WINDOW_SECS as u64).contains(&retry_after_secs));
            }
            other => panic!("expected the limit, got {other:?}"),
        }
        // Another email isn't affected.
        check(&pool, "b@example.test").await.unwrap();

        clear(&pool, "a@example.test").await.unwrap();
        check(&pool, "a@example.test").await.unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_finished_window_starts_over(pool: PgPool) {
        for _ in 0..MAX_FAILURES {
            record_failure(&pool, "a@example.test").await.unwrap();
        }
        assert!(check(&pool, "a@example.test").await.is_err());
        sqlx::query("UPDATE signin_failures SET window_started_at = now() - interval '16 minutes'")
            .execute(&pool)
            .await
            .unwrap();
        check(&pool, "a@example.test").await.unwrap();
        record_failure(&pool, "a@example.test").await.unwrap();
        let failures: i32 = sqlx::query_scalar("SELECT failures FROM signin_failures")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(failures, 1, "a new window");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn old_rows_are_pruned_and_emails_are_not_stored(pool: PgPool) {
        record_failure(&pool, "old@example.test").await.unwrap();
        sqlx::query("UPDATE signin_failures SET window_started_at = now() - interval '2 days'")
            .execute(&pool)
            .await
            .unwrap();
        record_failure(&pool, "new@example.test").await.unwrap();
        let rows: Vec<Vec<u8>> = sqlx::query_scalar("SELECT email_hash FROM signin_failures")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows, vec![email_hash("new@example.test")]);
        assert_eq!(rows[0].len(), 32, "a SHA-256, not the address");
    }
}
