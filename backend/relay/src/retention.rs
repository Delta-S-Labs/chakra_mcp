//! How long `usage_events` are kept.
//!
//! Every REST request and MCP tool call writes a usage event, inbox polls
//! included, so the table grows with traffic rather than with activity: a
//! pull agent polling every few seconds adds tens of thousands of rows a day.
//! A background task deletes events older than the retention window
//! (`USAGE_RETENTION_DAYS`, default 90; 0 keeps them forever), hourly and in
//! small batches, so it never holds many locks or competes with the writer.

use std::time::Duration;

use anyhow::Context;
use sqlx::PgPool;

/// Days kept when `USAGE_RETENTION_DAYS` is unset or empty.
pub const DEFAULT_USAGE_RETENTION_DAYS: u32 = 90;

/// How often old events are deleted.
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Rows deleted per statement.
const PRUNE_BATCH: i64 = 10_000;

/// Pause between batches, so a large backlog drains without hogging the
/// database.
const PRUNE_PAUSE: Duration = Duration::from_millis(100);

/// The retention window in days from `USAGE_RETENTION_DAYS`, or `None` to
/// keep events forever (`0`). Unset or empty means the default; anything else
/// that isn't a whole number of days is an error, so a typo stops startup
/// rather than silently keeping, or deleting, the wrong amount.
pub fn usage_retention_days(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<u32>> {
    let raw = get("USAGE_RETENTION_DAYS")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let Some(raw) = raw else {
        return Ok(Some(DEFAULT_USAGE_RETENTION_DAYS));
    };
    let days: u32 = raw
        .parse()
        .with_context(|| format!("USAGE_RETENTION_DAYS={raw:?} is not a whole number of days"))?;
    Ok((days > 0).then_some(days))
}

/// Deletes usage events older than `days`, in batches of `batch`. Returns how
/// many were deleted.
pub async fn prune_usage_events(db: &PgPool, days: u32, batch: i64) -> sqlx::Result<u64> {
    let days = i32::try_from(days).unwrap_or(i32::MAX);
    let mut deleted = 0;
    loop {
        let n = sqlx::query!(
            r#"
            DELETE FROM usage_events
             WHERE id IN (
                   SELECT id FROM usage_events
                    WHERE created_at < now() - make_interval(days => $1)
                    ORDER BY created_at
                    LIMIT $2)
            "#,
            days,
            batch,
        )
        .execute(db)
        .await?
        .rows_affected();
        deleted += n;
        if n < u64::try_from(batch).unwrap_or(u64::MAX) {
            return Ok(deleted);
        }
        tokio::time::sleep(PRUNE_PAUSE).await;
    }
}

/// Prunes old usage events now and every [`PRUNE_INTERVAL`]. A failed run is
/// logged and retried at the next tick.
pub fn spawn_usage_pruner(db: PgPool, days: u32) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PRUNE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            match prune_usage_events(&db, days, PRUNE_BATCH).await {
                Ok(0) => {}
                Ok(deleted) => tracing::info!(deleted, days, "deleted old usage events"),
                Err(e) => tracing::warn!(error = %e, "deleting old usage events failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(value: Option<&str>) -> impl Fn(&str) -> Option<String> + '_ {
        move |key| {
            (key == "USAGE_RETENTION_DAYS")
                .then(|| value.map(str::to_owned))
                .flatten()
        }
    }

    #[test]
    fn retention_defaults_to_90_days_and_zero_keeps_everything() {
        assert_eq!(usage_retention_days(env(None)).unwrap(), Some(90));
        assert_eq!(usage_retention_days(env(Some("  "))).unwrap(), Some(90));
        assert_eq!(usage_retention_days(env(Some("30"))).unwrap(), Some(30));
        assert_eq!(usage_retention_days(env(Some("0"))).unwrap(), None);
        for bad in ["-1", "90d", "1.5", "lots"] {
            assert!(usage_retention_days(env(Some(bad))).is_err(), "{bad}");
        }
    }

    async fn event_aged(db: &PgPool, days: i32) {
        sqlx::query(
            "INSERT INTO usage_events (id, created_at, surface, action, method, route, status_code, ok)
             VALUES ($1, now() - make_interval(days => $2), 'rest', 'GET /v1/inbox', 'GET', '/v1/inbox', 200, true)",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(days)
        .execute(db)
        .await
        .unwrap();
    }

    async fn ages(db: &PgPool) -> Vec<i32> {
        sqlx::query_scalar(
            "SELECT extract(day FROM now() - created_at)::int FROM usage_events ORDER BY created_at",
        )
        .fetch_all(db)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn prunes_only_events_past_the_window(db: PgPool) {
        for days in [120, 95, 91, 89, 1, 0] {
            event_aged(&db, days).await;
        }
        // A batch of 1 exercises the loop: three old rows, three statements.
        assert_eq!(prune_usage_events(&db, 90, 1).await.unwrap(), 3);
        assert_eq!(ages(&db).await, vec![89, 1, 0]);
        assert_eq!(prune_usage_events(&db, 90, 1).await.unwrap(), 0);
    }
}
