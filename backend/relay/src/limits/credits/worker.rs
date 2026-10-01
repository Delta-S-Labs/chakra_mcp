//! The credits worker: two loops beside the invocation path.
//!
//! - **Accounting** charges queued invocations and applies monthly free
//!   grants.
//! - **Switches** reloads who is blocked (and the rate overrides) into the
//!   [`CreditCache`] the invocation path reads.
//!
//! The loops run independently on the worker's own connections: a stalled
//! charge or grant can't stop the switches refreshing (which would make them
//! fail open), and the worker never takes a connection from the invocation
//! path. Every transaction gives up on a lock after a second. Nothing here
//! touches `relay_invocations`, so nothing here can make an invocation wait.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chakramcp_shared::telemetry::names;
use chrono::{Datelike, NaiveDate, Utc};
use metrics::{counter, gauge};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use super::{CreditCache, CreditsConfig, SET_LOCK_TIMEOUT};

/// `pg_try_advisory_xact_lock` key: only one instance does the accounting
/// at a time (row locks already prevent double charges; this avoids
/// instances racing for the same batch).
const ACCOUNTING_LOCK_KEY: i64 = 0x4352_4544_4954_5331; // "CREDITS1"

/// Queue rows drained per statement. Small enough that each transaction —
/// and the row locks it holds — stays short.
const CHARGE_BATCH: i64 = 1_000;

/// Upper bound on charging per accounting pass (never more than one tick),
/// so the pass gets to grants; a deep backlog continues next pass.
const CHARGE_BUDGET: Duration = Duration::from_secs(2);

/// Pause before restarting a loop that stopped.
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// Start the worker's two loops on a pool of their own. Each is restarted
/// if it ever stops, and the switches fail open when stale, so a dead loop
/// can't freeze anyone's block. Returns that pool, for the metrics sampler.
pub fn spawn_worker(db: &PgPool, cache: Arc<CreditCache>, cfg: CreditsConfig) -> PgPool {
    // Lets the staleness alert work out the switches' age from outside.
    gauge!(names::CREDITS_STALE_AFTER_SECONDS).set(cfg.stale_after().as_secs_f64());
    let db = worker_pool(db);
    let accounting_db = db.clone();
    supervise("accounting", RESTART_DELAY, move || {
        accounting_loop(accounting_db.clone(), cfg)
    });
    let switches_db = db.clone();
    supervise("switches", RESTART_DELAY, move || {
        switches_loop(switches_db.clone(), cache.clone(), cfg)
    });
    db
}

/// One connection per loop, with the app pool's connection settings.
fn worker_pool(db: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy_with(db.connect_options().as_ref().clone())
}

/// Keep `task` running: whenever it ends (it shouldn't) or panics, log it
/// and start a fresh one after `delay`.
fn supervise<F, Fut>(name: &'static str, delay: Duration, task: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            match tokio::spawn(task()).await {
                Ok(()) => tracing::error!(worker = name, "credits loop exited; restarting"),
                Err(e) => {
                    tracing::error!(worker = name, error = %e, "credits loop crashed; restarting")
                }
            }
            tokio::time::sleep(delay).await;
        }
    });
}

fn ticker(period: Duration) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

async fn accounting_loop(db: PgPool, cfg: CreditsConfig) {
    let mut ticker = ticker(cfg.sweep_interval);
    let mut last_depth = None;
    loop {
        ticker.tick().await;
        account(&db, &cfg, &mut last_depth).await;
    }
}

async fn switches_loop(db: PgPool, cache: Arc<CreditCache>, cfg: CreditsConfig) {
    let mut ticker = ticker(cfg.sweep_interval);
    let mut stale = false;
    loop {
        ticker.tick().await;
        refresh_switches(&db, &cache, &cfg, &mut stale).await;
    }
}

/// One accounting pass: charge the queue, then apply due grants. A failing
/// step is logged and never stops the other. With credits off, the pass
/// discards the queue instead.
pub(crate) async fn account(db: &PgPool, cfg: &CreditsConfig, last_depth: &mut Option<i64>) {
    if !cfg.enabled {
        return discard(db, cfg, last_depth).await;
    }
    let started = Instant::now();
    let budget = CHARGE_BUDGET.min(cfg.sweep_interval);
    let (charged, charge_end) = charge_pass(db, cfg.cost_per_invocation_mc, budget).await;
    let (granted, grants_ok) =
        match grant_due(db, cfg.default_monthly_free_mc, current_month()).await {
            Ok(granted) => (granted.unwrap_or(0), true),
            Err(e) => {
                tracing::warn!(error = %e, "credit grants failed");
                (0, false)
            }
        };
    let depth = match queue_depth(db).await {
        Ok(depth) => Some(depth),
        Err(e) => {
            tracing::warn!(error = %e, "credit queue depth check failed");
            None
        }
    };
    record_accounting(charged, charge_end, grants_ok, depth);

    let took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if charged.rows > 0 || granted > 0 {
        tracing::info!(
            charged = charged.rows,
            wallet_debits = charged.debits,
            granted,
            queue = ?depth,
            took_ms,
            "credits accounting"
        );
    } else {
        tracing::debug!(queue = ?depth, took_ms, "credits accounting (idle)");
    }
    if let (Some(previous), Some(now)) = (*last_depth, depth) {
        if now > previous && now >= CHARGE_BATCH {
            tracing::warn!(
                queue = now,
                previous,
                "credit charge queue is growing; the worker is behind"
            );
        }
    }
    *last_depth = depth;
}

/// With credits off: drop queued charges in batches and make no grants.
/// Nothing is billed and no wallet changes, but the queue still drains, so it
/// can't grow, and turning credits on later starts from an empty queue.
async fn discard(db: &PgPool, cfg: &CreditsConfig, last_depth: &mut Option<i64>) {
    let started = Instant::now();
    let budget = CHARGE_BUDGET.min(cfg.sweep_interval);
    let mut discarded = 0;
    let end = loop {
        match discard_batch(db).await {
            Ok(Some(rows)) => {
                discarded += rows;
                if rows < CHARGE_BATCH || started.elapsed() > budget {
                    break PassEnd::Done;
                }
            }
            Ok(None) => break PassEnd::LockHeld,
            Err(e) => {
                tracing::warn!(error = %e, "discarding queued credit charges failed");
                break PassEnd::Failed;
            }
        }
    };
    let depth = match queue_depth(db).await {
        Ok(depth) => Some(depth),
        Err(e) => {
            tracing::warn!(error = %e, "credit queue depth check failed");
            None
        }
    };
    record_accounting(ChargeTotals::default(), end, true, depth);
    if discarded > 0 {
        tracing::debug!(discarded, "credits are off: discarded queued charges");
    }
    *last_depth = depth;
}

/// One discard batch in its own transaction; `None` if another instance
/// holds the accounting lock.
async fn discard_batch(db: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    let Some(mut tx) = begin_accounting(db).await? else {
        return Ok(None);
    };
    let result = sqlx::query_scalar!(
        r#"
        WITH drained AS (
            DELETE FROM credit_charge_queue
             WHERE invocation_id = ANY(ARRAY(SELECT invocation_id FROM credit_charge_queue
                                              ORDER BY invocation_id
                                              LIMIT $1
                                              FOR UPDATE SKIP LOCKED))
            RETURNING 1
        )
        SELECT count(*) AS "drained!" FROM drained
        "#,
        CHARGE_BATCH,
    )
    .fetch_one(&mut *tx)
    .await;
    finish(tx, result).await.map(Some)
}

/// Reload the switches. A refresh that fails — or takes longer than a tick,
/// which counts as failing, so a stall (a dead connection, a crawling query)
/// can't go unnoticed — leaves the last snapshot in place until it goes
/// stale; from then on nobody is blocked (fail open), which is logged as an
/// error because enforcement is off until a refresh succeeds. `stale`
/// carries that state between calls.
pub(crate) async fn refresh_switches(
    db: &PgPool,
    cache: &CreditCache,
    cfg: &CreditsConfig,
    stale: &mut bool,
) {
    let before = cache.blocked_count();
    let refresh = cache.refresh(db, cfg.block_below_mc());
    let refreshed = match tokio::time::timeout(cfg.sweep_interval, refresh).await {
        Ok(result) => result.map_err(|e| e.to_string()),
        Err(_) => Err(format!("timed out after {:?}", cfg.sweep_interval)),
    };
    record_refresh(cache, refreshed.is_ok());
    match refreshed {
        Ok(()) => {
            let blocked = cache.blocked_count();
            if std::mem::take(stale) {
                tracing::info!(blocked, "credit switches refreshed; enforcement is back on");
            } else if blocked != before {
                tracing::info!(blocked, "credit switches changed");
            }
        }
        Err(e) if cache.is_stale() => {
            if std::mem::replace(stale, true) {
                tracing::warn!(error = %e, "credit switch refresh failed; still stale");
            } else {
                tracing::error!(
                    error = %e,
                    "credit switches are stale: nobody is blocked until a refresh succeeds"
                );
            }
        }
        Err(e) => tracing::warn!(error = %e, "credit switch refresh failed"),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChargeTotals {
    /// Queue rows drained and charged.
    pub rows: i64,
    /// Wallet debits applied (one per account per batch).
    pub debits: i64,
}

/// How a charging pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassEnd {
    /// Drained the queue, or spent the budget.
    Done,
    /// Another instance holds the accounting lock.
    LockHeld,
    /// A batch failed.
    Failed,
}

/// Drain the queue in short batches until it's empty, another instance holds
/// the lock, a batch fails, or `budget` is spent.
async fn charge_pass(db: &PgPool, cost_mc: i64, budget: Duration) -> (ChargeTotals, PassEnd) {
    let started = Instant::now();
    let mut totals = ChargeTotals::default();
    let end = loop {
        match charge_batch(db, cost_mc).await {
            Ok(Some(batch)) => {
                totals.rows += batch.rows;
                totals.debits += batch.debits;
                if batch.rows < CHARGE_BATCH || started.elapsed() > budget {
                    break PassEnd::Done;
                }
            }
            Ok(None) => break PassEnd::LockHeld, // another instance is accounting
            Err(e) => {
                tracing::warn!(error = %e, "credit charge failed");
                break PassEnd::Failed;
            }
        }
    };
    (totals, end)
}

#[cfg(test)]
async fn charge_all(db: &PgPool, cost_mc: i64, budget: Duration) -> ChargeTotals {
    charge_pass(db, cost_mc, budget).await.0
}

/// An accounting pass's metrics: `error` if charging or grants failed,
/// `skipped` if another instance held the lock and nothing was charged.
fn record_accounting(charged: ChargeTotals, end: PassEnd, grants_ok: bool, depth: Option<i64>) {
    let result = match end {
        PassEnd::Failed => "error",
        _ if !grants_ok => "error",
        PassEnd::LockHeld if charged.rows == 0 => "skipped",
        _ => "ok",
    };
    counter!(names::CREDITS_ACCOUNTING_RUNS_TOTAL, "result" => result).increment(1);
    counter!(names::CREDITS_CHARGES_TOTAL).increment(u64::try_from(charged.rows).unwrap_or(0));
    if let Some(depth) = depth {
        gauge!(names::CREDITS_QUEUE_DEPTH).set(depth as f64);
    }
}

/// A switch refresh's metrics.
fn record_refresh(cache: &CreditCache, ok: bool) {
    counter!(
        names::CREDITS_SWITCH_REFRESHES_TOTAL,
        "result" => if ok { "ok" } else { "error" }
    )
    .increment(1);
    if ok {
        gauge!(names::CREDITS_SWITCHES_LAST_REFRESH_TIMESTAMP_SECONDS)
            .set(Utc::now().timestamp_millis() as f64 / 1000.0);
        gauge!(names::CREDITS_BLOCKED_ACCOUNTS).set(cache.blocked_count() as f64);
    }
    gauge!(names::CREDITS_SWITCHES_STALE).set(if cache.is_stale() { 1.0 } else { 0.0 });
}

/// One batch in its own transaction; `None` if another instance holds the lock.
async fn charge_batch(db: &PgPool, cost_mc: i64) -> Result<Option<ChargeTotals>, sqlx::Error> {
    let Some(mut tx) = begin_accounting(db).await? else {
        return Ok(None);
    };
    let result = charge_rows(&mut tx, CHARGE_BATCH, cost_mc).await;
    finish(tx, result).await.map(Some)
}

/// Drain up to `batch` queue rows, record a charge for each, and debit each
/// account's wallet by its total — one statement, so charges and balances
/// can never diverge. A wallet is created at an account's first charge; a
/// charge for an account deleted since is recorded as 0 with no debit.
///
/// `= ANY(ARRAY(…))` runs the row pick once and then deletes by primary key;
/// with `IN (…)` the planner can choose a hash join over the whole queue,
/// which makes draining a backlog quadratic.
async fn charge_rows(
    conn: &mut PgConnection,
    batch: i64,
    cost_mc: i64,
) -> Result<ChargeTotals, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        WITH drained AS (
            DELETE FROM credit_charge_queue
             WHERE invocation_id = ANY(ARRAY(SELECT invocation_id FROM credit_charge_queue
                                              ORDER BY invocation_id
                                              LIMIT $1
                                              FOR UPDATE SKIP LOCKED))
            RETURNING invocation_id, account_id
        ), charged AS (
            INSERT INTO invocation_charges (invocation_id, account_id, cost_mc)
            SELECT d.invocation_id, d.account_id,
                   CASE WHEN EXISTS (SELECT 1 FROM accounts a WHERE a.id = d.account_id)
                        THEN $2::bigint ELSE 0 END
              FROM drained d
            ON CONFLICT (invocation_id) DO NOTHING
            RETURNING account_id, cost_mc
        ), totals AS (
            SELECT account_id, SUM(cost_mc)::bigint AS total
              FROM charged
             WHERE cost_mc > 0
             GROUP BY account_id
        ), debited AS (
            INSERT INTO credit_wallets (account_id, balance_mc, updated_at)
            SELECT account_id, -total, now() FROM totals
            ON CONFLICT (account_id) DO UPDATE
               SET balance_mc = credit_wallets.balance_mc + EXCLUDED.balance_mc,
                   updated_at = now()
            RETURNING account_id
        )
        SELECT (SELECT count(*) FROM drained) AS "drained!",
               (SELECT count(*) FROM debited) AS "debits!"
        "#,
        batch,
        cost_mc,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(ChargeTotals {
        rows: row.drained,
        debits: row.debits,
    })
}

/// Apply every due monthly grant in its own transaction; `None` if another
/// instance holds the lock.
async fn grant_due(
    db: &PgPool,
    default_grant_mc: i64,
    period: NaiveDate,
) -> Result<Option<u64>, sqlx::Error> {
    let Some(mut tx) = begin_accounting(db).await? else {
        return Ok(None);
    };
    let result = grant_rows(&mut tx, period, default_grant_mc).await;
    finish(tx, result).await.map(Some)
}

/// Grant every wallet not yet granted for `period` (a first-of-month date):
/// one month's grant, or one per missed month (capped at 12) after a worker
/// outage. Each grant is a `free_grant` ledger row carrying the exact
/// resulting balance. Returns how many wallets were granted.
///
/// Safe under concurrency even without the advisory lock: `FOR UPDATE` plus
/// the re-check in `upd` means an overlapping run grants nothing twice.
async fn grant_rows(
    conn: &mut PgConnection,
    period: NaiveDate,
    default_grant_mc: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        WITH due AS (
            SELECT account_id,
                   (CASE WHEN free_grant_period IS NULL THEN 1
                         ELSE LEAST(12,
                                ((EXTRACT(YEAR FROM $1::date) - EXTRACT(YEAR FROM free_grant_period)) * 12
                               + (EXTRACT(MONTH FROM $1::date) - EXTRACT(MONTH FROM free_grant_period)))::int)
                    END)::bigint * COALESCE(monthly_free_grant_mc, $2::bigint) AS delta
              FROM credit_wallets
             WHERE free_grant_period IS NULL OR free_grant_period < $1::date
               FOR UPDATE
        ), upd AS (
            UPDATE credit_wallets w
               SET balance_mc = w.balance_mc + due.delta,
                   free_grant_period = $1::date,
                   updated_at = now()
              FROM due
             WHERE w.account_id = due.account_id
               AND (w.free_grant_period IS NULL OR w.free_grant_period < $1::date)
            RETURNING w.account_id, due.delta, w.balance_mc
        )
        INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc, metadata)
        SELECT account_id, delta, 'free_grant', balance_mc, jsonb_build_object('period', $1::date)
          FROM upd
        "#,
        period,
        default_grant_mc,
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected())
}

/// Begin an accounting transaction: bound its lock waits, then take the
/// accounting lock (try-only, as its own statement, so it's held before any
/// statement whose snapshot matters). `None` if another instance holds it.
async fn begin_accounting(
    db: &PgPool,
) -> Result<Option<Transaction<'static, Postgres>>, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(SET_LOCK_TIMEOUT).execute(&mut *tx).await?;
    let locked = sqlx::query_scalar!(
        r#"SELECT pg_try_advisory_xact_lock($1) AS "locked!""#,
        ACCOUNTING_LOCK_KEY,
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(locked.then_some(tx))
}

/// Commit on success. On failure roll back right away: a dropped
/// transaction's ROLLBACK is sent later, by a background task when its
/// connection returns to the pool, and until then it would keep the
/// accounting lock from the pass's next step.
async fn finish<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, sqlx::Error>,
) -> Result<T, sqlx::Error> {
    match result {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(e) => {
            if let Err(rollback) = tx.rollback().await {
                tracing::warn!(error = %rollback, "credits rollback failed");
            }
            Err(e)
        }
    }
}

/// Rows waiting to be charged (for the logs). Lock-bounded like everything
/// else here: a table lock on the queue must not stall the accounting loop.
async fn queue_depth(db: &PgPool) -> Result<i64, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(SET_LOCK_TIMEOUT).execute(&mut *tx).await?;
    let depth = sqlx::query_scalar!(r#"SELECT count(*) AS "depth!" FROM credit_charge_queue"#)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(depth)
}

/// The grant period for now: the first of the current UTC month.
pub(crate) fn current_month() -> NaiveDate {
    let today = Utc::now().date_naive();
    NaiveDate::from_ymd_opt(today.year(), today.month(), 1).expect("the 1st is always a valid day")
}

#[cfg(test)]
mod tests {
    use chrono::Months;
    use uuid::Uuid;

    use super::*;
    use crate::telemetry::testing::Recorded;

    const COST: i64 = 100;
    const GRANT: i64 = 100_000;

    async fn seed_account(pool: &PgPool) -> Uuid {
        let owner = Uuid::now_v7();
        sqlx::query("INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'worker-test')")
            .bind(owner)
            .bind(format!("worker-{}@t.local", owner.simple()))
            .execute(pool)
            .await
            .unwrap();
        let account = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
             VALUES ($1, $2, 'worker-test', 'individual', $3)",
        )
        .bind(account)
        .bind(format!("worker-{}", &account.simple().to_string()[..12]))
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
        account
    }

    async fn enqueue_id(pool: &PgPool, invocation: Uuid, account: Uuid) {
        sqlx::query("INSERT INTO credit_charge_queue (invocation_id, account_id) VALUES ($1, $2)")
            .bind(invocation)
            .bind(account)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn enqueue(pool: &PgPool, account: Uuid, n: usize) {
        for _ in 0..n {
            enqueue_id(pool, Uuid::now_v7(), account).await;
        }
    }

    async fn balance(pool: &PgPool, account: Uuid) -> Option<i64> {
        sqlx::query_scalar("SELECT balance_mc FROM credit_wallets WHERE account_id = $1")
            .bind(account)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    async fn count(pool: &PgPool, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    /// A wallet granted this month holding `balance_mc`.
    async fn granted_wallet(pool: &PgPool, account: Uuid, balance_mc: i64) {
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period) VALUES ($1, $2, $3)",
        )
        .bind(account)
        .bind(balance_mc)
        .bind(current_month())
        .execute(pool)
        .await
        .unwrap();
    }

    /// A second pool on the same test database with room for real
    /// concurrency (the test pool may be too small).
    async fn concurrent_pool(pool: &PgPool) -> PgPool {
        PgPoolOptions::new()
            .max_connections(4)
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn charging_drains_the_queue_and_debits_wallets(pool: PgPool) {
        let (a, b) = (seed_account(&pool).await, seed_account(&pool).await);
        enqueue(&pool, a, 3).await;
        enqueue(&pool, b, 1).await;

        let charged = charge_all(&pool, COST, CHARGE_BUDGET).await;
        assert_eq!(charged.rows, 4);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM credit_charge_queue").await,
            0
        );
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM invocation_charges WHERE cost_mc = 100"
            )
            .await,
            4
        );
        // Wallets are created at the first charge, not yet granted.
        assert_eq!(balance(&pool, a).await, Some(-300));
        assert_eq!(balance(&pool, b).await, Some(-100));

        // Nothing left to do: a second pass is a no-op.
        assert_eq!(
            charge_all(&pool, COST, CHARGE_BUDGET).await,
            ChargeTotals::default()
        );
        assert_eq!(balance(&pool, a).await, Some(-300));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_backlog_drains_in_batches(pool: PgPool) {
        let account = seed_account(&pool).await;
        sqlx::query(
            "INSERT INTO credit_charge_queue (invocation_id, account_id)
             SELECT gen_random_uuid(), $1 FROM generate_series(1, 2500)",
        )
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();

        // A generous budget: this is about batching, not the time cap.
        let charged = charge_all(&pool, COST, Duration::from_secs(60)).await;
        assert_eq!(charged.rows, 2500);
        assert_eq!(charged.debits, 3, "one debit per account per batch");
        assert_eq!(balance(&pool, account).await, Some(-2500 * COST));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_deleted_account_is_charged_zero(pool: PgPool) {
        let gone = Uuid::now_v7(); // never existed / since deleted
        enqueue(&pool, gone, 2).await;

        assert_eq!(charge_all(&pool, COST, CHARGE_BUDGET).await.rows, 2);
        let recorded: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM invocation_charges WHERE account_id = $1 AND cost_mc = 0",
        )
        .bind(gone)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            recorded, 2,
            "charges recorded at 0 so nothing is left unaccounted"
        );
        assert_eq!(
            balance(&pool, gone).await,
            None,
            "no wallet for a missing account"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_second_drain_skips_locked_rows_instead_of_waiting(pool: PgPool) {
        // Explicit, ordered ids: the first drain's batch is exactly the
        // first account's rows, so the two drains share no wallet either.
        let (first_acct, second_acct) = (seed_account(&pool).await, seed_account(&pool).await);
        for i in 1..=10u128 {
            let account = if i <= 5 { first_acct } else { second_acct };
            enqueue_id(&pool, Uuid::from_u128(i), account).await;
        }

        let wide = concurrent_pool(&pool).await;
        let mut first = wide.begin().await.unwrap();
        assert_eq!(charge_rows(&mut first, 5, COST).await.unwrap().rows, 5);

        // `first` still holds its rows. Without the advisory lock, a second
        // drain must take the rest without waiting on them: SKIP LOCKED.
        let mut second = wide.begin().await.unwrap();
        sqlx::query("SET LOCAL lock_timeout = '200ms'")
            .execute(&mut *second)
            .await
            .unwrap();
        let charged = charge_rows(&mut second, CHARGE_BATCH, COST)
            .await
            .expect("skips the locked rows rather than waiting on them");
        assert_eq!(charged.rows, 5);
        second.commit().await.unwrap();
        first.commit().await.unwrap();

        assert_eq!(
            count(&pool, "SELECT count(*) FROM invocation_charges").await,
            10
        );
        assert_eq!(balance(&pool, first_acct).await, Some(-5 * COST));
        assert_eq!(balance(&pool, second_acct).await, Some(-5 * COST));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_second_instance_skips_while_one_is_accounting(pool: PgPool) {
        let account = seed_account(&pool).await;
        enqueue(&pool, account, 3).await;
        let wide = concurrent_pool(&pool).await;

        // Instance A is mid-pass, holding the accounting lock.
        let a = begin_accounting(&wide)
            .await
            .unwrap()
            .expect("lock is free");
        // Instance B doesn't wait for it; it skips this pass.
        let started = Instant::now();
        assert_eq!(charge_batch(&wide, COST).await.unwrap(), None);
        assert_eq!(
            grant_due(&wide, GRANT, current_month()).await.unwrap(),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        // A whole pass by B while A holds the lock is counted as skipped.
        let m = Recorded::start();
        super::account(&wide, &CreditsConfig::default(), &mut None).await;
        let runs = names::CREDITS_ACCOUNTING_RUNS_TOTAL;
        assert_eq!(m.counter(runs, &[("result", "skipped")]), 1);
        assert_eq!(m.counter(names::CREDITS_CHARGES_TOTAL, &[]), 0);
        a.rollback().await.unwrap();

        // Lock released: B's next pass does the work.
        assert_eq!(charge_all(&wide, COST, CHARGE_BUDGET).await.rows, 3);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_locked_wallet_costs_a_pass_not_a_hang(pool: PgPool) {
        let cfg = CreditsConfig::default();
        // A charge is queued for a wallet another session has locked (an
        // admin mid-edit, say)...
        let busy = seed_account(&pool).await;
        granted_wallet(&pool, busy, 1_000).await;
        enqueue(&pool, busy, 1).await;
        // ...while another account is due its first grant.
        let newcomer = seed_account(&pool).await;
        set_period(&pool, newcomer, None).await;

        let wide = concurrent_pool(&pool).await;
        let mut admin = wide.begin().await.unwrap();
        sqlx::query("SELECT 1 FROM credit_wallets WHERE account_id = $1 FOR UPDATE")
            .bind(busy)
            .execute(&mut *admin)
            .await
            .unwrap();

        // The charge gives up after the lock timeout; the grant still lands.
        tokio::time::timeout(Duration::from_secs(5), account(&wide, &cfg, &mut None))
            .await
            .expect("a held row lock must not stall the pass");
        assert_eq!(
            count(&pool, "SELECT count(*) FROM credit_charge_queue").await,
            1,
            "left for the next pass"
        );
        assert_eq!(balance(&pool, newcomer).await, Some(GRANT));

        // Row locks don't hold up the switches either.
        let cache = CreditCache::default();
        tokio::time::timeout(
            Duration::from_secs(2),
            cache.refresh(&wide, cfg.cost_per_invocation_mc),
        )
        .await
        .expect("refresh doesn't wait on row locks")
        .unwrap();
        assert!(!cache.is_stale());

        // Once the lock is gone, the next pass charges it.
        admin.rollback().await.unwrap();
        account(&wide, &cfg, &mut None).await;
        assert_eq!(
            count(&pool, "SELECT count(*) FROM credit_charge_queue").await,
            0
        );
        assert_eq!(balance(&pool, busy).await, Some(1_000 - COST));
    }

    async fn set_period(pool: &PgPool, account: Uuid, period: Option<NaiveDate>) {
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, free_grant_period) VALUES ($1, $2)
             ON CONFLICT (account_id) DO UPDATE SET free_grant_period = EXCLUDED.free_grant_period",
        )
        .bind(account)
        .bind(period)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn grant_now(pool: &PgPool) -> u64 {
        grant_due(pool, GRANT, current_month())
            .await
            .unwrap()
            .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn grants_follow_the_calendar(pool: PgPool) {
        let this_month = current_month();
        let fresh = seed_account(&pool).await;
        let last_month = seed_account(&pool).await;
        let dormant = seed_account(&pool).await;
        set_period(&pool, fresh, None).await;
        set_period(
            &pool,
            last_month,
            this_month.checked_sub_months(Months::new(1)),
        )
        .await;
        set_period(
            &pool,
            dormant,
            this_month.checked_sub_months(Months::new(34)),
        )
        .await;

        assert_eq!(grant_now(&pool).await, 3);
        assert_eq!(
            balance(&pool, fresh).await,
            Some(GRANT),
            "never granted → one month"
        );
        assert_eq!(
            balance(&pool, last_month).await,
            Some(GRANT),
            "rollover → one month"
        );
        assert_eq!(
            balance(&pool, dormant).await,
            Some(12 * GRANT),
            "a 34-month gap is capped at 12"
        );

        // Same month again: nothing is due.
        assert_eq!(grant_now(&pool).await, 0);
        assert_eq!(balance(&pool, fresh).await, Some(GRANT));

        // Each grant is a ledger row with the exact resulting balance.
        let ledger: (i64, i64) = sqlx::query_as(
            "SELECT delta_mc, balance_after_mc FROM credit_ledger
              WHERE account_id = $1 AND reason = 'free_grant'",
        )
        .bind(dormant)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ledger, (12 * GRANT, 12 * GRANT));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn per_account_grant_overrides_the_default(pool: PgPool) {
        let account = seed_account(&pool).await;
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, monthly_free_grant_mc) VALUES ($1, 5000)",
        )
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
        grant_now(&pool).await;
        assert_eq!(balance(&pool, account).await, Some(5000));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn overlapping_grant_runs_never_double_grant(pool: PgPool) {
        let mut accounts = Vec::new();
        for _ in 0..10 {
            let account = seed_account(&pool).await;
            set_period(&pool, account, None).await;
            accounts.push(account);
        }

        // Two overlapping runs without the advisory lock.
        let wide = concurrent_pool(&pool).await;
        let run = |db: PgPool| async move {
            let mut tx = db.begin().await.unwrap();
            let granted = grant_rows(&mut tx, current_month(), GRANT).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            tx.commit().await.unwrap();
            granted
        };
        let (first, second) = tokio::join!(run(wide.clone()), run(wide.clone()));

        assert_eq!(first + second, 10);
        assert_eq!(count(&pool, "SELECT count(*) FROM credit_ledger").await, 10);
        for account in accounts {
            assert_eq!(balance(&pool, account).await, Some(GRANT));
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_pass_charges_and_grants_then_the_switches_flip(pool: PgPool) {
        let m = Recorded::start();
        let cfg = CreditsConfig::default();
        let cache = CreditCache::default();

        // A new account's first invocations: its wallet is created by the
        // charge and granted in the same pass, so it's never falsely blocked.
        let newcomer = seed_account(&pool).await;
        enqueue(&pool, newcomer, 2).await;

        // An account granted this month that has spent almost everything.
        let spent = seed_account(&pool).await;
        granted_wallet(&pool, spent, 150).await;
        enqueue(&pool, spent, 1).await;

        account(&pool, &cfg, &mut None).await;
        refresh_switches(&pool, &cache, &cfg, &mut false).await;

        assert_eq!(balance(&pool, newcomer).await, Some(GRANT - 2 * COST));
        assert!(!cache.is_blocked(newcomer));
        assert_eq!(balance(&pool, spent).await, Some(50));
        assert!(
            cache.is_blocked(spent),
            "50 mc can't pay for another invocation"
        );

        assert_eq!(
            m.counter(names::CREDITS_ACCOUNTING_RUNS_TOTAL, &[("result", "ok")]),
            1
        );
        assert_eq!(m.counter(names::CREDITS_CHARGES_TOTAL, &[]), 3);
        assert_eq!(m.gauge(names::CREDITS_QUEUE_DEPTH, &[]), Some(0.0));
        assert_eq!(
            m.counter(names::CREDITS_SWITCH_REFRESHES_TOTAL, &[("result", "ok")]),
            1
        );
        assert_eq!(m.gauge(names::CREDITS_BLOCKED_ACCOUNTS, &[]), Some(1.0));
        assert_eq!(m.gauge(names::CREDITS_SWITCHES_STALE, &[]), Some(0.0));
        let refreshed_at = m
            .gauge(names::CREDITS_SWITCHES_LAST_REFRESH_TIMESTAMP_SECONDS, &[])
            .expect("set on a successful refresh");
        let age = Utc::now().timestamp_millis() as f64 / 1000.0 - refreshed_at;
        assert!((0.0..60.0).contains(&age), "{age}");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn with_credits_off_the_queue_drains_and_nobody_pays_or_is_blocked(pool: PgPool) {
        let m = Recorded::start();
        let cfg = CreditsConfig {
            enabled: false,
            ..CreditsConfig::default()
        };
        let cache = CreditCache::default();

        // A wallet left from a time credits were on, out of credits.
        let spent = seed_account(&pool).await;
        granted_wallet(&pool, spent, 50).await;
        enqueue(&pool, spent, 3).await;
        let newcomer = seed_account(&pool).await;
        enqueue(&pool, newcomer, 2).await;

        account(&pool, &cfg, &mut None).await;
        refresh_switches(&pool, &cache, &cfg, &mut false).await;

        assert_eq!(
            count(&pool, "SELECT count(*) FROM credit_charge_queue").await,
            0
        );
        assert_eq!(
            count(&pool, "SELECT count(*) FROM invocation_charges").await,
            0
        );
        assert_eq!(
            count(&pool, "SELECT count(*) FROM credit_ledger").await,
            0,
            "no grants"
        );
        assert_eq!(balance(&pool, spent).await, Some(50), "untouched");
        assert_eq!(balance(&pool, newcomer).await, None, "no wallet created");
        assert!(
            !cache.is_blocked(spent),
            "nobody is blocked with credits off"
        );

        assert_eq!(
            m.counter(names::CREDITS_ACCOUNTING_RUNS_TOTAL, &[("result", "ok")]),
            1
        );
        assert_eq!(m.counter(names::CREDITS_CHARGES_TOTAL, &[]), 0);
        assert_eq!(m.gauge(names::CREDITS_QUEUE_DEPTH, &[]), Some(0.0));
        assert_eq!(m.gauge(names::CREDITS_BLOCKED_ACCOUNTS, &[]), Some(0.0));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn failed_refreshes_fail_open_once_stale_then_recover(pool: PgPool) {
        let m = Recorded::start();
        let broke = seed_account(&pool).await;
        granted_wallet(&pool, broke, 0).await;
        let cfg = CreditsConfig::default();
        let cache = CreditCache::new(Duration::from_millis(500));
        let mut stale = false;
        refresh_switches(&pool, &cache, &cfg, &mut stale).await;
        assert!(cache.is_blocked(broke) && !stale);

        // The database goes away: the last snapshot stands until it's stale...
        let gone = concurrent_pool(&pool).await;
        gone.close().await;
        refresh_switches(&gone, &cache, &cfg, &mut stale).await;
        assert!(cache.is_blocked(broke) && !stale, "still fresh");
        tokio::time::sleep(Duration::from_millis(600)).await;
        refresh_switches(&gone, &cache, &cfg, &mut stale).await;
        assert!(stale, "flagged (and logged) as stale");
        assert!(!cache.is_blocked(broke), "stale switches fail open");
        assert_eq!(m.gauge(names::CREDITS_SWITCHES_STALE, &[]), Some(1.0));

        // ...and the next good refresh restores enforcement.
        refresh_switches(&pool, &cache, &cfg, &mut stale).await;
        assert!(!stale && cache.is_blocked(broke));
        assert_eq!(m.gauge(names::CREDITS_SWITCHES_STALE, &[]), Some(0.0));
        let refreshes = names::CREDITS_SWITCH_REFRESHES_TOTAL;
        assert_eq!(m.counter(refreshes, &[("result", "ok")]), 2);
        assert_eq!(m.counter(refreshes, &[("result", "error")]), 2);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_stalled_refresh_counts_as_failed(pool: PgPool) {
        let cfg = CreditsConfig {
            sweep_interval: Duration::from_millis(200),
            ..CreditsConfig::default()
        };
        let cache = CreditCache::new(cfg.stale_after());
        let mut stale = false;

        // A table lock on the wallets (a migration mid-ALTER, say) stalls the
        // refresh; it's given up at the tick, before the lock timeout.
        let wide = concurrent_pool(&pool).await;
        let mut ddl = wide.begin().await.unwrap();
        sqlx::query("LOCK TABLE credit_wallets IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *ddl)
            .await
            .unwrap();
        let started = Instant::now();
        refresh_switches(&wide, &cache, &cfg, &mut stale).await;
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "took {:?}",
            started.elapsed()
        );
        assert!(
            stale,
            "never loaded, now stalled: flagged (and logged) as stale"
        );

        ddl.rollback().await.unwrap();
        refresh_switches(&wide, &cache, &cfg, &mut stale).await;
        assert!(!stale && !cache.is_stale());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn the_queue_depth_check_bounds_its_lock_wait(pool: PgPool) {
        let wide = concurrent_pool(&pool).await;
        let mut ddl = wide.begin().await.unwrap();
        sqlx::query("LOCK TABLE credit_charge_queue IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *ddl)
            .await
            .unwrap();
        let depth = tokio::time::timeout(Duration::from_secs(5), queue_depth(&wide))
            .await
            .expect("gives up at the lock timeout");
        assert!(depth.is_err());

        ddl.rollback().await.unwrap();
        assert_eq!(queue_depth(&wide).await.unwrap(), 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn balances_reconcile_with_ledger_and_charges(pool: PgPool) {
        let cfg = CreditsConfig::default();
        let (a, b) = (seed_account(&pool).await, seed_account(&pool).await);
        enqueue(&pool, a, 7).await;
        account(&pool, &cfg, &mut None).await;
        enqueue(&pool, a, 3).await;
        enqueue(&pool, b, 4).await;
        account(&pool, &cfg, &mut None).await;

        for account in [a, b] {
            // SUM(bigint) is numeric in Postgres: cast back for decoding.
            let reconciled: i64 = sqlx::query_scalar(
                "SELECT (COALESCE((SELECT SUM(delta_mc) FROM credit_ledger WHERE account_id = $1), 0)
                       - COALESCE((SELECT SUM(cost_mc) FROM invocation_charges WHERE account_id = $1), 0))::bigint",
            )
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(balance(&pool, account).await, Some(reconciled));
        }
    }

    #[tokio::test]
    async fn a_loop_that_crashes_or_exits_is_restarted_after_a_pause() {
        let delay = Duration::from_millis(20);
        let starts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let log = starts.clone();
        supervise("test", delay, move || {
            let run = {
                let mut starts = log.lock().unwrap();
                starts.push(Instant::now());
                starts.len() - 1
            };
            let tx = tx.clone();
            async move {
                match run {
                    0 => panic!("first run crashes"),
                    1 => {} // second run returns early
                    _ => {
                        tx.send(()).unwrap();
                        std::future::pending::<()>().await; // third runs on
                    }
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("restarted after a crash and after an exit")
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let starts = starts.lock().unwrap();
        assert_eq!(starts.len(), 3, "a running loop is left alone");
        // Paused before each restart, including after a clean exit, so a
        // loop that keeps returning can't spin.
        assert!(starts[1] - starts[0] >= delay);
        assert!(starts[2] - starts[1] >= delay);
    }
}
