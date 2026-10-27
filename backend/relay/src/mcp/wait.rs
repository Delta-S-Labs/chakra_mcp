//! Waiting for an invocation's result inside one MCP tool call (MCP and
//! discovery spec §6.6), so a chat client gets the answer instead of an id
//! to poll. It only reads the invocation's row by primary key; nothing on
//! the invoke path changes.

use std::time::Duration;

use tokio::time::Instant;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use crate::state::RelayState;

/// Polls every 500 ms for the first 5 seconds, then every second.
const FAST_POLL: Duration = Duration::from_millis(500);
const FAST_FOR: Duration = Duration::from_secs(5);
const SLOW_POLL: Duration = Duration::from_secs(1);

/// Whether a status is where waiting stops: a result, a refusal, or a
/// request held for its owner's approval.
pub fn settled(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "failed" | "rejected" | "timeout" | "awaiting_consent"
    )
}

/// Wait up to `timeout` for invocation `id` to settle, and return its
/// status then. With every waiting slot taken (`MCP_MAX_WAITERS`) it
/// returns the status at once.
pub async fn wait_for(state: &RelayState, id: Uuid, timeout: Duration) -> Result<String, ApiError> {
    let mut status = status_of(state, id).await?;
    if settled(&status) || timeout.is_zero() {
        return Ok(status);
    }
    let Ok(_slot) = state.waiters.clone().try_acquire_owned() else {
        return Ok(status);
    };
    let started = Instant::now();
    let deadline = started + timeout;
    loop {
        let poll = if started.elapsed() < FAST_FOR {
            FAST_POLL
        } else {
            SLOW_POLL
        };
        let now = Instant::now();
        if now >= deadline {
            return Ok(status);
        }
        tokio::time::sleep(poll.min(deadline - now)).await;
        status = status_of(state, id).await?;
        if settled(&status) {
            return Ok(status);
        }
    }
}

async fn status_of(state: &RelayState, id: Uuid) -> Result<String, ApiError> {
    sqlx::query_scalar!("SELECT status FROM relay_invocations WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn invocation(pool: &PgPool, status: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO relay_invocations (id, capability_name, status) VALUES ($1, 'x', $2)",
        )
        .bind(id)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    fn state(pool: &PgPool, waiters: usize) -> RelayState {
        RelayState::new(
            pool.clone(),
            crate::handlers::invoke::legacy_v01_contract_tests::config(),
        )
        .with_mcp(crate::mcp::McpConfig {
            max_waiters: waiters,
            ..Default::default()
        })
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_when_the_result_arrives(pool: PgPool) {
        let id = invocation(&pool, "pending").await;
        let (s, p) = (state(&pool, 1), pool.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            sqlx::query("UPDATE relay_invocations SET status = 'succeeded' WHERE id = $1")
                .bind(id)
                .execute(&p)
                .await
                .unwrap();
        });
        let started = Instant::now();
        assert_eq!(
            wait_for(&s, id, Duration::from_secs(10)).await.unwrap(),
            "succeeded"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn held_requests_and_deadlines_return(pool: PgPool) {
        let s = state(&pool, 1);
        let held = invocation(&pool, "awaiting_consent").await;
        assert_eq!(
            wait_for(&s, held, Duration::from_secs(10)).await.unwrap(),
            "awaiting_consent"
        );
        let pending = invocation(&pool, "pending").await;
        let started = Instant::now();
        assert_eq!(
            wait_for(&s, pending, Duration::from_millis(600))
                .await
                .unwrap(),
            "pending"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn no_free_waiter_returns_at_once(pool: PgPool) {
        let s = state(&pool, 1);
        let _taken = s.waiters.clone().try_acquire_owned().unwrap();
        let pending = invocation(&pool, "pending").await;
        let started = Instant::now();
        assert_eq!(
            wait_for(&s, pending, Duration::from_secs(10))
                .await
                .unwrap(),
            "pending"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
