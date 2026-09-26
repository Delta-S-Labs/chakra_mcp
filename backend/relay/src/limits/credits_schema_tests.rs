//! Migration 0034 (credits, expand): the plan-tier guard and the new tables.
//!
//! Runtime `sqlx::query` only, no macros: the guard tests touch
//! `plans`/`plan_id`, which migration 0035 drops, and runtime queries stay
//! out of the `.sqlx` cache.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

/// Last migration before the credit tables (#312's grant purpose).
const BEFORE_CREDITS: i64 = 33;
/// The credits expand migration.
const CREDITS_EXPAND: i64 = 34;

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

/// An account on the default plan. Valid before and after 0035.
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

/// An account on a named plan. Only valid before 0035 drops `plans`.
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
