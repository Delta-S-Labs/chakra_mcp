//! Usage limits — per-account rate limiting (Redis) + monthly invocation
//! quotas (Postgres), governed by named plans.
//!
//! Design: `docs/specs/2026-07-23-usage-quotas-rate-limiting-design.md`.
//!
//! This module is the shared primitive both the A2A and legacy/MCP invocation
//! surfaces call in PR 3:
//!   * [`check`] — read-only rate + quota gate, keyed on the caller's account.
//!   * [`quota::increment`] — bump the monthly counter in the row-write txn.
//!
//! Enforcement is not wired into any surface in this PR (PR 2). It is built,
//! constructed at relay startup, and unit-tested in isolation.

pub mod quota;
pub mod rate;

pub use rate::{RateLimiter, RateOutcome};

use sqlx::PgPool;
use uuid::Uuid;

/// Combined result of the two checks at an invocation surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitOutcome {
    Allowed,
    RateLimited,
    QuotaExceeded,
}

/// The limit knobs resolved from an account's plan.
#[derive(Debug, Clone, Copy)]
pub struct PlanLimits {
    pub rate_limit_per_min: i32,
    /// `None` = unlimited.
    pub monthly_invocation_quota: Option<i64>,
}

/// Resolve the plan limits for `account_id`. Every account has a plan
/// (`accounts.plan_id` is NOT NULL DEFAULT free as of migration 0032), so a
/// missing row is a real error rather than a silent fall-through.
pub async fn resolve_plan(db: &PgPool, account_id: Uuid) -> Result<PlanLimits, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT p.rate_limit_per_min, p.monthly_invocation_quota
          FROM accounts a
          JOIN plans p ON p.id = a.plan_id
         WHERE a.id = $1
        "#,
        account_id,
    )
    .fetch_one(db)
    .await?;
    Ok(PlanLimits {
        rate_limit_per_min: row.rate_limit_per_min,
        monthly_invocation_quota: row.monthly_invocation_quota,
    })
}

/// Read-only rate + quota gate for a caller account. Runs the (cheap) rate
/// check first, then the quota check. Records one rate hit as a side effect
/// of the rate check (fixed-window `INCR`); the durable quota counter is only
/// bumped by [`quota::increment`] on a successful invocation, never here.
///
/// The caller decides what to do with a non-`Allowed` outcome (deny vs
/// shadow-log) — this function neither enforces nor logs.
pub async fn check(
    db: &PgPool,
    limiter: &RateLimiter,
    account_id: Uuid,
) -> Result<LimitOutcome, sqlx::Error> {
    let plan = resolve_plan(db, account_id).await?;

    if limiter.check(account_id, plan.rate_limit_per_min).await == RateOutcome::Limited {
        return Ok(LimitOutcome::RateLimited);
    }

    if let Some(quota) = plan.monthly_invocation_quota {
        if quota::current_month(db, account_id).await? >= quota {
            return Ok(LimitOutcome::QuotaExceeded);
        }
    }

    Ok(LimitOutcome::Allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_account_on_plan(pool: &PgPool, plan_name: &str) -> Uuid {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'limit-test')",
            user_id,
            format!("limit-{}@t.local", user_id.simple()),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id, plan_id)
               VALUES ($1, $2, 'limit-test', 'individual', $3, (SELECT id FROM plans WHERE name = $4))"#,
            account_id,
            format!("limit-{}", &account_id.simple().to_string()[..12]),
            user_id,
            plan_name,
        )
        .execute(pool)
        .await
        .unwrap();
        account_id
    }

    fn counting_limiter() -> RateLimiter {
        RateLimiter::Counting(std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::new(),
        )))
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn resolve_plan_reads_the_accounts_plan(pool: PgPool) {
        let acct = seed_account_on_plan(&pool, "pro").await;
        let limits = resolve_plan(&pool, acct).await.unwrap();
        assert_eq!(limits.rate_limit_per_min, 600);
        assert_eq!(limits.monthly_invocation_quota, Some(50000));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn default_account_resolves_to_free(pool: PgPool) {
        // Seeded without an explicit plan → migration default (free).
        let user_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'free-test')",
            user_id,
            format!("free-{}@t.local", user_id.simple()),
        )
        .execute(&pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'free-test', 'individual', $3)"#,
            account_id,
            format!("free-{}", &account_id.simple().to_string()[..12]),
            user_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        let limits = resolve_plan(&pool, account_id).await.unwrap();
        assert_eq!(limits.rate_limit_per_min, 60);
        assert_eq!(limits.monthly_invocation_quota, Some(1000));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn check_allows_under_both_limits(pool: PgPool) {
        let acct = seed_account_on_plan(&pool, "free").await;
        assert_eq!(
            check(&pool, &RateLimiter::Noop, acct).await.unwrap(),
            LimitOutcome::Allowed
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn check_flags_rate_before_quota(pool: PgPool) {
        // free = 60/min. The counting limiter trips on the 61st hit.
        let acct = seed_account_on_plan(&pool, "free").await;
        let rl = counting_limiter();
        for _ in 0..60 {
            assert_eq!(
                check(&pool, &rl, acct).await.unwrap(),
                LimitOutcome::Allowed
            );
        }
        assert_eq!(
            check(&pool, &rl, acct).await.unwrap(),
            LimitOutcome::RateLimited
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn check_flags_quota_when_counter_at_limit(pool: PgPool) {
        // free monthly quota = 1000; push the counter to the cap.
        let acct = seed_account_on_plan(&pool, "free").await;
        sqlx::query!(
            r#"INSERT INTO usage_counters (account_id, period_start, invocations)
               VALUES ($1, date_trunc('month', now() AT TIME ZONE 'UTC')::date, 1000)"#,
            acct,
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            check(&pool, &RateLimiter::Noop, acct).await.unwrap(),
            LimitOutcome::QuotaExceeded
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn enterprise_quota_is_unlimited(pool: PgPool) {
        let acct = seed_account_on_plan(&pool, "enterprise").await;
        // Even with a huge counter, NULL quota never trips.
        sqlx::query!(
            r#"INSERT INTO usage_counters (account_id, period_start, invocations)
               VALUES ($1, date_trunc('month', now() AT TIME ZONE 'UTC')::date, 999999999)"#,
            acct,
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            check(&pool, &RateLimiter::Noop, acct).await.unwrap(),
            LimitOutcome::Allowed
        );
    }
}
