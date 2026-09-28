//! The credits worker: charges queued invocations, applies monthly free
//! grants, and refreshes the in-memory switches. It runs beside the
//! invocation path and never touches `relay_invocations`, so nothing it does
//! can make an invocation wait.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::task::JoinHandle;

use super::{CreditCache, CreditsConfig};

/// `pg_try_advisory_xact_lock` key: only one instance does the accounting
/// at a time (row locks already prevent double charges; this avoids
/// instances racing for the same batch).
const ACCOUNTING_LOCK_KEY: i64 = 0x4352_4544_4954_5331; // "CREDITS1"

/// Queue rows drained per statement. Small enough that each transaction —
/// and the row locks it holds — stays short.
const CHARGE_BATCH: i64 = 1_000;

/// Upper bound on charging per tick; a deep backlog continues next tick.
const CHARGE_BUDGET: Duration = Duration::from_secs(2);

/// Spawn the worker. It runs forever; if it ever panics it's restarted, so
/// blocking can't silently freeze (the switches also fail open when stale).
pub fn spawn_worker(db: PgPool, cache: Arc<CreditCache>, cfg: CreditsConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let run = tokio::spawn(run(db.clone(), cache.clone(), cfg));
            if let Err(e) = run.await {
                tracing::error!(error = %e, "credits worker stopped unexpectedly; restarting");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    })
}

async fn run(db: PgPool, cache: Arc<CreditCache>, cfg: CreditsConfig) {
    let mut ticker = tokio::time::interval(cfg.sweep_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_depth = None;
    loop {
        ticker.tick().await;
        tick(&db, &cache, &cfg, &mut last_depth).await;
    }
}

/// One pass: charge the queue, apply due grants, refresh the switches.
/// A failing step is logged and never stops the others.
pub(crate) async fn tick(
    db: &PgPool,
    cache: &CreditCache,
    cfg: &CreditsConfig,
    last_depth: &mut Option<i64>,
) {
    let started = Instant::now();
    let charged = charge_all(db, cfg.cost_per_invocation_mc).await;
    let granted = match grant_due(db, cfg.default_monthly_free_mc, current_month()).await {
        Ok(granted) => granted.unwrap_or(0),
        Err(e) => {
            tracing::warn!(error = %e, "credit grants failed");
            0
        }
    };
    if let Err(e) = cache.refresh(db, cfg.cost_per_invocation_mc).await {
        tracing::warn!(error = %e, "credit switch refresh failed");
    }
    let depth = match queue_depth(db).await {
        Ok(depth) => Some(depth),
        Err(e) => {
            tracing::warn!(error = %e, "credit queue depth check failed");
            None
        }
    };

    let blocked = cache.blocked_count();
    let took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if charged.rows > 0 || granted > 0 {
        tracing::info!(
            charged = charged.rows,
            wallet_debits = charged.debits,
            granted,
            blocked,
            queue = ?depth,
            took_ms,
            "credits tick"
        );
    } else {
        tracing::debug!(blocked, queue = ?depth, took_ms, "credits tick (idle)");
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChargeTotals {
    /// Queue rows drained and charged.
    pub rows: i64,
    /// Wallet debits applied (one per account per batch).
    pub debits: i64,
}

/// Drain the queue in short batches until it's empty, another instance holds
/// the lock, or this tick's budget is spent.
async fn charge_all(db: &PgPool, cost_mc: i64) -> ChargeTotals {
    let started = Instant::now();
    let mut totals = ChargeTotals::default();
    loop {
        match charge_batch(db, cost_mc).await {
            Ok(Some(batch)) => {
                totals.rows += batch.rows;
                totals.debits += batch.debits;
                if batch.rows < CHARGE_BATCH || started.elapsed() > CHARGE_BUDGET {
                    break;
                }
            }
            Ok(None) => break, // another instance is doing the accounting
            Err(e) => {
                tracing::warn!(error = %e, "credit charge failed");
                break;
            }
        }
    }
    totals
}

/// One batch in its own transaction; `None` if another instance holds the lock.
async fn charge_batch(db: &PgPool, cost_mc: i64) -> Result<Option<ChargeTotals>, sqlx::Error> {
    let mut tx = db.begin().await?;
    if !try_accounting_lock(&mut tx).await? {
        return Ok(None);
    }
    let totals = charge_rows(&mut tx, CHARGE_BATCH, cost_mc).await?;
    tx.commit().await?;
    Ok(Some(totals))
}

/// Drain up to `batch` queue rows, record a charge for each, and debit each
/// account's wallet by its total — one statement, so charges and balances
/// can never diverge. A wallet is created at an account's first charge; a
/// charge for an account deleted since is recorded as 0 with no debit.
async fn charge_rows(
    conn: &mut PgConnection,
    batch: i64,
    cost_mc: i64,
) -> Result<ChargeTotals, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        WITH drained AS (
            DELETE FROM credit_charge_queue
             WHERE invocation_id IN (SELECT invocation_id FROM credit_charge_queue
                                      ORDER BY invocation_id
                                      LIMIT $1
                                      FOR UPDATE SKIP LOCKED)
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
    let mut tx = db.begin().await?;
    if !try_accounting_lock(&mut tx).await? {
        return Ok(None);
    }
    let granted = grant_rows(&mut tx, period, default_grant_mc).await?;
    tx.commit().await?;
    Ok(Some(granted))
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

/// Take the accounting lock for the current transaction, as its own
/// statement (so it's held before any statement whose snapshot matters).
async fn try_accounting_lock(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT pg_try_advisory_xact_lock($1) AS "locked!""#,
        ACCOUNTING_LOCK_KEY,
    )
    .fetch_one(&mut *conn)
    .await
}

async fn queue_depth(db: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT count(*) AS "depth!" FROM credit_charge_queue"#)
        .fetch_one(db)
        .await
}

/// The grant period for now: the first of the current UTC month.
pub(crate) fn current_month() -> NaiveDate {
    let today = Utc::now().date_naive();
    NaiveDate::from_ymd_opt(today.year(), today.month(), 1).expect("the 1st is always a valid day")
}

#[cfg(test)]
mod tests {
    use chrono::Months;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

    use super::*;

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

    async fn enqueue(pool: &PgPool, account: Uuid, n: usize) {
        for _ in 0..n {
            sqlx::query(
                "INSERT INTO credit_charge_queue (invocation_id, account_id) VALUES ($1, $2)",
            )
            .bind(Uuid::now_v7())
            .bind(account)
            .execute(pool)
            .await
            .unwrap();
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

        let charged = charge_all(&pool, COST).await;
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
        assert_eq!(charge_all(&pool, COST).await, ChargeTotals::default());
        assert_eq!(balance(&pool, a).await, Some(-300));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_deleted_account_is_charged_zero(pool: PgPool) {
        let gone = Uuid::now_v7(); // never existed / since deleted
        enqueue(&pool, gone, 2).await;

        assert_eq!(charge_all(&pool, COST).await.rows, 2);
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
    async fn concurrent_drains_never_double_charge(pool: PgPool) {
        let account = seed_account(&pool).await;
        enqueue(&pool, account, 50).await;

        // Two overlapping transactions draining without the advisory lock:
        // SKIP LOCKED alone must keep them from charging the same row.
        let wide = concurrent_pool(&pool).await;
        let drain = |db: PgPool| async move {
            let mut tx = db.begin().await.unwrap();
            let charged = charge_rows(&mut tx, CHARGE_BATCH, COST).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await; // hold the locks
            tx.commit().await.unwrap();
            charged.rows
        };
        let (first, second) = tokio::join!(drain(wide.clone()), drain(wide.clone()));

        assert_eq!(first + second, 50);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM invocation_charges").await,
            50
        );
        assert_eq!(balance(&pool, account).await, Some(-50 * COST));
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
    async fn a_tick_charges_grants_and_flips_the_switches(pool: PgPool) {
        let cfg = CreditsConfig::default();
        let cache = CreditCache::default();

        // A new account's first invocations: its wallet is created by the
        // charge and granted in the same tick, so it's never falsely blocked.
        let newcomer = seed_account(&pool).await;
        enqueue(&pool, newcomer, 2).await;

        // An account granted this month that has spent almost everything.
        let spent = seed_account(&pool).await;
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period) VALUES ($1, 150, $2)",
        )
        .bind(spent)
        .bind(current_month())
        .execute(&pool)
        .await
        .unwrap();
        enqueue(&pool, spent, 1).await;

        tick(&pool, &cache, &cfg, &mut None).await;

        assert_eq!(balance(&pool, newcomer).await, Some(GRANT - 2 * COST));
        assert!(!cache.is_blocked(newcomer));
        assert_eq!(balance(&pool, spent).await, Some(50));
        assert!(
            cache.is_blocked(spent),
            "50 mc can't pay for another invocation"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn balances_reconcile_with_ledger_and_charges(pool: PgPool) {
        let cfg = CreditsConfig::default();
        let cache = CreditCache::default();
        let (a, b) = (seed_account(&pool).await, seed_account(&pool).await);
        enqueue(&pool, a, 7).await;
        tick(&pool, &cache, &cfg, &mut None).await;
        enqueue(&pool, a, 3).await;
        enqueue(&pool, b, 4).await;
        tick(&pool, &cache, &cfg, &mut None).await;

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
}
