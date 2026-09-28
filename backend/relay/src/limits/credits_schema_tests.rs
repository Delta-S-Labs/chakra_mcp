//! Migrations 0034 (credits, expand), 0035 (hardening) and 0036 (contract):
//! the plan-tier guard, the new tables and their constraints, and the drop of
//! the plan tables.
//!
//! Runtime `sqlx::query` only, no macros: the guard tests touch
//! `plans`/`plan_id`, which migration 0036 drops, and runtime queries stay
//! out of the `.sqlx` cache.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

/// Last migration before the credit tables (#312's grant purpose).
const BEFORE_CREDITS: i64 = 33;
/// The credits expand migration.
const CREDITS_EXPAND: i64 = 34;
/// The hardening migration, the last one before the contract.
const CREDITS_HARDENING: i64 = 35;
/// The contract migration: drops `plans`, `accounts.plan_id`, `usage_counters`.
const CREDITS_CONTRACT: i64 = 36;

async fn seed_user(pool: &PgPool) -> Uuid {
    let user_id = Uuid::now_v7();
    sqlx::query("INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'credits-test')")
        .bind(user_id)
        .bind(format!("credits-{}@t.local", user_id.simple()))
        .execute(pool)
        .await
        .unwrap();
    user_id
}

fn account_slug(account_id: Uuid) -> String {
    format!("credits-{}", &account_id.simple().to_string()[..12])
}

/// An account on the default plan. Valid before and after 0036.
async fn seed_account(pool: &PgPool) -> Uuid {
    let owner = seed_user(pool).await;
    let account_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
         VALUES ($1, $2, 'credits-test', 'individual', $3)",
    )
    .bind(account_id)
    .bind(account_slug(account_id))
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    account_id
}

/// An account on a named plan. Only valid before 0036 drops `plans`.
async fn seed_account_on_plan(pool: &PgPool, plan: &str) {
    let owner = seed_user(pool).await;
    let account_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id, plan_id)
         VALUES ($1, $2, 'credits-test', 'individual', $3,
                 (SELECT id FROM plans WHERE name = $4))",
    )
    .bind(account_id)
    .bind(account_slug(account_id))
    .bind(owner)
    .bind(plan)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = false)]
async fn expand_refuses_accounts_on_non_free_plans(pool: PgPool) {
    let migrator = sqlx::migrate!("../migrations");
    migrator.run_to(BEFORE_CREDITS, &pool).await.unwrap();
    seed_account_on_plan(&pool, "free").await;
    seed_account_on_plan(&pool, "pro").await;

    assert!(
        migrator.run_to(CREDITS_EXPAND, &pool).await.is_err(),
        "the guard must refuse an account on a non-free plan"
    );

    // The migration's transaction rolled back: nothing created, nothing recorded.
    let created: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_name = 'credit_wallets'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(created, 0);
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(CREDITS_EXPAND)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0);
}

#[sqlx::test(migrations = false)]
async fn expand_runs_and_backfills_nothing(pool: PgPool) {
    let migrator = sqlx::migrate!("../migrations");
    migrator.run_to(BEFORE_CREDITS, &pool).await.unwrap();
    seed_account_on_plan(&pool, "free").await;
    migrator.run_to(CREDITS_EXPAND, &pool).await.unwrap();

    // Wallets appear at an account's first charge, not at migration time.
    let wallets: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_wallets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(wallets, 0);
}

async fn insert_ledger(
    pool: &PgPool,
    reason: &str,
    external_ref: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO credit_ledger (account_id, delta_mc, reason, external_ref, balance_after_mc)
         VALUES ($1, 1000, $2, $3, 1000)",
    )
    .bind(Uuid::now_v7()) // no FK to accounts: history outlives the account
    .bind(reason)
    .bind(external_ref)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrations = "../migrations")]
async fn purchases_need_a_unique_payment_ref(pool: PgPool) {
    assert!(
        insert_ledger(&pool, "purchase", None).await.is_err(),
        "a purchase needs a payment id"
    );
    insert_ledger(&pool, "purchase", Some("pay_1"))
        .await
        .unwrap();
    assert!(
        insert_ledger(&pool, "purchase", Some("pay_1"))
            .await
            .is_err(),
        "a retried webhook must not credit the same payment twice"
    );

    // Only purchases are deduplicated.
    insert_ledger(&pool, "admin_grant", None).await.unwrap();
    insert_ledger(&pool, "adjustment", Some("pay_1"))
        .await
        .unwrap();
    insert_ledger(&pool, "adjustment", Some("pay_1"))
        .await
        .unwrap();

    assert!(
        insert_ledger(&pool, "refund", None).await.is_err(),
        "refunds are manual adjustments, not a ledger reason"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn wallets_belong_to_an_account(pool: PgPool) {
    let orphan = sqlx::query("INSERT INTO credit_wallets (account_id) VALUES ($1)")
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await;
    assert!(orphan.is_err(), "a wallet needs an existing account");

    // A new wallet starts empty, never granted, and limited.
    let account = seed_account(&pool).await;
    sqlx::query("INSERT INTO credit_wallets (account_id) VALUES ($1)")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    let wallet: (i64, Option<NaiveDate>, bool) = sqlx::query_as(
        "SELECT balance_mc, free_grant_period, unlimited FROM credit_wallets WHERE account_id = $1",
    )
    .bind(account)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(wallet, (0, None, false));

    // The wallet goes away with its account.
    sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_wallets WHERE account_id = $1")
        .bind(account)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrations = "../migrations")]
async fn an_invocation_is_queued_and_charged_at_most_once(pool: PgPool) {
    // Neither table has FKs, so arbitrary ids are fine.
    let invocation = Uuid::now_v7();
    let account = Uuid::now_v7();

    let enqueue = || {
        sqlx::query("INSERT INTO credit_charge_queue (invocation_id, account_id) VALUES ($1, $2)")
            .bind(invocation)
            .bind(account)
    };
    enqueue().execute(&pool).await.unwrap();
    assert!(enqueue().execute(&pool).await.is_err(), "queued twice");

    let charge = || {
        sqlx::query(
            "INSERT INTO invocation_charges (invocation_id, account_id, cost_mc) VALUES ($1, $2, 100)",
        )
        .bind(invocation)
        .bind(account)
    };
    charge().execute(&pool).await.unwrap();
    assert!(charge().execute(&pool).await.is_err(), "charged twice");
}

async fn set_overrides(
    pool: &PgPool,
    account: Uuid,
    rate_limit_per_min: Option<i32>,
    monthly_free_grant_mc: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO credit_wallets (account_id, rate_limit_per_min, monthly_free_grant_mc)
         VALUES ($1, $2, $3)
         ON CONFLICT (account_id) DO UPDATE
            SET rate_limit_per_min = EXCLUDED.rate_limit_per_min,
                monthly_free_grant_mc = EXCLUDED.monthly_free_grant_mc",
    )
    .bind(account)
    .bind(rate_limit_per_min)
    .bind(monthly_free_grant_mc)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrations = "../migrations")]
async fn wallet_overrides_reject_nonsense(pool: PgPool) {
    let account = seed_account(&pool).await;
    // NULL means "use the default"; a zero grant is a legitimate override.
    set_overrides(&pool, account, None, None).await.unwrap();
    set_overrides(&pool, account, Some(1), Some(0))
        .await
        .unwrap();

    for rate in [0, -5] {
        assert!(
            set_overrides(&pool, account, Some(rate), None)
                .await
                .is_err(),
            "a rate limit of {rate} would refuse every call"
        );
    }
    assert!(
        set_overrides(&pool, account, None, Some(-1)).await.is_err(),
        "a negative grant would debit the account every month"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn vacuum_never_truncates_the_queue(pool: PgPool) {
    // Truncation takes an ACCESS EXCLUSIVE lock that would stall the
    // invocation path's queue inserts.
    let options: Vec<String> = sqlx::query_scalar(
        "SELECT unnest(reloptions) FROM pg_class
          WHERE oid = 'credit_charge_queue'::regclass",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        options.iter().any(|o| o == "vacuum_truncate=false"),
        "{options:?}"
    );
}

async fn plan_objects_left(pool: &PgPool) -> (Option<String>, Option<String>, i64) {
    sqlx::query_as(
        "SELECT to_regclass('plans')::text, to_regclass('usage_counters')::text,
                (SELECT count(*) FROM information_schema.columns
                  WHERE table_name = 'accounts' AND column_name = 'plan_id')",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../migrations")]
async fn contract_drops_the_plan_tables(pool: PgPool) {
    assert_eq!(plan_objects_left(&pool).await, (None, None, 0));
}

#[sqlx::test(migrations = false)]
async fn contract_keeps_accounts_and_their_credits(pool: PgPool) {
    let migrator = sqlx::migrate!("../migrations");
    migrator.run_to(CREDITS_HARDENING, &pool).await.unwrap();
    // An account (on the default plan), its old monthly counter, and a wallet.
    let account = seed_account(&pool).await;
    sqlx::query("INSERT INTO usage_counters (account_id, period_start, invocations) VALUES ($1, '2026-09-01', 42)")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO credit_wallets (account_id, balance_mc) VALUES ($1, 1234)")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();

    migrator.run_to(CREDITS_CONTRACT, &pool).await.unwrap();

    assert_eq!(plan_objects_left(&pool).await, (None, None, 0));
    let balance: i64 =
        sqlx::query_scalar("SELECT balance_mc FROM credit_wallets WHERE account_id = $1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(balance, 1234, "the account and its wallet survive");
}

/// `accounts` is read on the invocation path. While the contract migration
/// waits for its lock, readers must not queue up behind it: it tries for
/// 100 ms at a time and backs off in between.
#[sqlx::test(migrations = false)]
async fn contract_never_parks_readers_behind_its_lock(pool: PgPool) {
    use std::time::{Duration, Instant};

    let migrator = sqlx::migrate!("../migrations");
    migrator.run_to(CREDITS_HARDENING, &pool).await.unwrap();
    seed_account(&pool).await;
    let wide = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();

    // A long read of `accounts` is in flight when the migration starts...
    let mut long_read = wide.begin().await.unwrap();
    sqlx::query("SELECT count(*) FROM accounts")
        .execute(&mut *long_read)
        .await
        .unwrap();
    let migration = tokio::spawn({
        let wide = wide.clone();
        async move {
            sqlx::migrate!("../migrations")
                .run_to(CREDITS_CONTRACT, &wide)
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ...and a new read arriving meanwhile waits at most one short attempt,
    // not for the long read to finish.
    let mut reader = wide.acquire().await.unwrap();
    for _ in 0..3 {
        let started = Instant::now();
        sqlx::query("SELECT count(*) FROM accounts")
            .execute(&mut *reader)
            .await
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "a reader waited {:?} behind the migration",
            started.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(reader);
    assert!(!migration.is_finished(), "still waiting for the long read");

    // Once the long read ends, the migration gets its lock and finishes.
    tokio::time::sleep(Duration::from_millis(500)).await;
    long_read.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), migration)
        .await
        .expect("retries until the lock is free")
        .unwrap()
        .unwrap();
    assert_eq!(plan_objects_left(&pool).await, (None, None, 0));
}
