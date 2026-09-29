//! The in-memory switches the invocation path reads. Only the worker's
//! refresh writes them; lookups never touch the database.

use std::collections::{HashMap, HashSet};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant};

use sqlx::PgPool;
use uuid::Uuid;

/// Which accounts are blocked, plus per-account rate-limit overrides, as of
/// the last refresh.
#[derive(Debug)]
pub struct CreditCache {
    snapshot: RwLock<Snapshot>,
    stale_after: Duration,
}

#[derive(Debug, Default)]
struct Snapshot {
    blocked: HashSet<Uuid>,
    rate_overrides: HashMap<Uuid, i32>,
    refreshed_at: Option<Instant>,
}

impl Default for CreditCache {
    /// Never refreshed, so nobody is blocked: what tests and the default
    /// `RelayState` get.
    fn default() -> Self {
        Self::new(Duration::from_secs(15))
    }
}

impl CreditCache {
    /// `stale_after`: how long a refresh stays authoritative.
    pub fn new(stale_after: Duration) -> Self {
        Self {
            snapshot: RwLock::new(Snapshot::default()),
            stale_after,
        }
    }

    /// Whether `account` is out of credits. Fails open: if the switches
    /// were never loaded or haven't been refreshed recently (the worker is
    /// down), nobody is blocked.
    pub fn is_blocked(&self, account: Uuid) -> bool {
        let snapshot = self.snapshot.read().unwrap_or_else(PoisonError::into_inner);
        self.is_fresh(&snapshot) && snapshot.blocked.contains(&account)
    }

    /// Whether the switches are unknown — never loaded, or not refreshed
    /// within `stale_after` — so nobody is blocked.
    pub fn is_stale(&self) -> bool {
        let snapshot = self.snapshot.read().unwrap_or_else(PoisonError::into_inner);
        !self.is_fresh(&snapshot)
    }

    fn is_fresh(&self, snapshot: &Snapshot) -> bool {
        snapshot
            .refreshed_at
            .is_some_and(|at| at.elapsed() <= self.stale_after)
    }

    /// The account's per-minute rate limit: its override, else `default`.
    /// The last known overrides stay in force even if the cache goes stale.
    pub fn rate_limit_for(&self, account: Uuid, default: i32) -> i32 {
        let snapshot = self.snapshot.read().unwrap_or_else(PoisonError::into_inner);
        snapshot
            .rate_overrides
            .get(&account)
            .copied()
            .unwrap_or(default)
    }

    /// How many accounts are currently blocked (for the worker's logs).
    pub fn blocked_count(&self) -> usize {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .blocked
            .len()
    }

    /// Swap in a new snapshot and mark it fresh.
    pub fn replace(&self, blocked: HashSet<Uuid>, rate_overrides: HashMap<Uuid, i32>) {
        let mut snapshot = self
            .snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *snapshot = Snapshot {
            blocked,
            rate_overrides,
            refreshed_at: Some(Instant::now()),
        };
    }

    /// Reload the switches from `credit_wallets`. An account is blocked when
    /// it can't afford one more invocation — unless it's unlimited or has
    /// never been granted (a wallet the worker created moments ago must not
    /// be blocked before its first grant lands). Keep this in step with
    /// `chakramcp_shared::credits::is_blocked`, which the app shows owners.
    pub async fn refresh(
        &self,
        db: &PgPool,
        cost_per_invocation_mc: i64,
    ) -> Result<(), sqlx::Error> {
        // Plain reads never wait on row locks, but they do queue behind a
        // table lock (DDL); give up after the lock timeout instead. And cut a
        // crawling read off server-side, so it isn't left running after the
        // worker has given up on it.
        let mut tx = db.begin().await?;
        sqlx::query(super::SET_LOCK_TIMEOUT)
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        let blocked = sqlx::query_scalar!(
            r#"
            SELECT account_id FROM credit_wallets
             WHERE NOT unlimited
               AND free_grant_period IS NOT NULL
               AND balance_mc < $1
            "#,
            cost_per_invocation_mc,
        )
        .fetch_all(&mut *tx)
        .await?;
        let rate_overrides = sqlx::query!(
            r#"
            SELECT account_id, rate_limit_per_min AS "rate_limit_per_min!"
              FROM credit_wallets
             WHERE rate_limit_per_min IS NOT NULL
            "#,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        self.replace(
            blocked.into_iter().collect(),
            rate_overrides
                .into_iter()
                .map(|r| (r.account_id, r.rate_limit_per_min))
                .collect(),
        );
        Ok(())
    }
}

/// Test helper: give `account` an exhausted wallet (granted this month,
/// nothing left) and return a cache refreshed to see it as blocked.
#[cfg(test)]
pub(crate) async fn exhausted_cache(pool: &PgPool, account: Uuid) -> std::sync::Arc<CreditCache> {
    sqlx::query(
        "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period)
         VALUES ($1, 0, date_trunc('month', now() AT TIME ZONE 'UTC')::date)",
    )
    .bind(account)
    .execute(pool)
    .await
    .unwrap();
    let cache = CreditCache::default();
    cache
        .refresh(pool, super::CreditsConfig::default().cost_per_invocation_mc)
        .await
        .unwrap();
    assert!(
        cache.is_blocked(account),
        "precondition: account must be blocked"
    );
    std::sync::Arc::new(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_refreshed_blocks_nobody() {
        let cache = CreditCache::default();
        assert!(cache.is_stale());
        assert!(!cache.is_blocked(Uuid::now_v7()));
    }

    #[test]
    fn a_fresh_snapshot_blocks_exactly_its_accounts() {
        let cache = CreditCache::default();
        let (out, fine) = (Uuid::now_v7(), Uuid::now_v7());
        cache.replace(HashSet::from([out]), HashMap::new());
        assert!(cache.is_blocked(out));
        assert!(!cache.is_blocked(fine));
        assert_eq!(cache.blocked_count(), 1);
    }

    #[test]
    fn a_stale_snapshot_fails_open() {
        let cache = CreditCache::new(Duration::from_millis(1));
        let out = Uuid::now_v7();
        cache.replace(HashSet::from([out]), HashMap::new());
        std::thread::sleep(Duration::from_millis(10));
        assert!(cache.is_stale());
        assert!(
            !cache.is_blocked(out),
            "a dead worker must not freeze blocks"
        );
    }

    #[test]
    fn rate_overrides_fall_back_to_the_default() {
        let cache = CreditCache::default();
        let (custom, plain) = (Uuid::now_v7(), Uuid::now_v7());
        cache.replace(HashSet::new(), HashMap::from([(custom, 600)]));
        assert_eq!(cache.rate_limit_for(custom, 60), 600);
        assert_eq!(cache.rate_limit_for(plain, 60), 60);
    }

    async fn wallet(pool: &PgPool, balance: i64, granted: bool, unlimited: bool) -> Uuid {
        let owner = Uuid::now_v7();
        sqlx::query("INSERT INTO users (id, email, display_name) VALUES ($1, $2, 'cache-test')")
            .bind(owner)
            .bind(format!("cache-{}@t.local", owner.simple()))
            .execute(pool)
            .await
            .unwrap();
        let account = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
             VALUES ($1, $2, 'cache-test', 'individual', $3)",
        )
        .bind(account)
        .bind(format!("cache-{}", &account.simple().to_string()[..12]))
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period, unlimited)
             VALUES ($1, $2, CASE WHEN $3 THEN date_trunc('month', now())::date END, $4)",
        )
        .bind(account)
        .bind(balance)
        .bind(granted)
        .bind(unlimited)
        .execute(pool)
        .await
        .unwrap();
        account
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn refresh_blocks_only_granted_limited_wallets_that_cant_pay(pool: PgPool) {
        let cost = 100;
        let broke = wallet(&pool, 99, true, false).await;
        let negative = wallet(&pool, -500, true, false).await;
        let can_pay = wallet(&pool, 100, true, false).await;
        let unlimited = wallet(&pool, -500, true, true).await;
        let never_granted = wallet(&pool, -100, false, false).await;

        let cache = CreditCache::default();
        cache.refresh(&pool, cost).await.unwrap();

        assert!(cache.is_blocked(broke));
        assert!(cache.is_blocked(negative));
        assert!(!cache.is_blocked(can_pay));
        assert!(!cache.is_blocked(unlimited));
        assert!(!cache.is_blocked(never_granted));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn refresh_loads_rate_overrides(pool: PgPool) {
        let custom = wallet(&pool, 1_000, true, false).await;
        let plain = wallet(&pool, 1_000, true, false).await;
        sqlx::query("UPDATE credit_wallets SET rate_limit_per_min = 7 WHERE account_id = $1")
            .bind(custom)
            .execute(&pool)
            .await
            .unwrap();

        let cache = CreditCache::default();
        cache.refresh(&pool, 100).await.unwrap();

        assert_eq!(cache.rate_limit_for(custom, 60), 7);
        assert_eq!(cache.rate_limit_for(plain, 60), 60);
    }
}
