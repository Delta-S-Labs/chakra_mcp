//! Background refresh job for push-mode Agent Cards.
//!
//! The refresh job picks one stale push agent per tick (default every
//! 60s) and re-fetches its upstream Agent Card. Multi-replica safe:
//! the claim query uses `FOR UPDATE SKIP LOCKED` so two relay
//! processes can run the job concurrently without picking the same
//! row.
//!
//! Backoff semantics:
//!
//! - `agents.agent_card_last_attempted_at` is bumped at *claim time*
//!   regardless of whether the fetch succeeds. A failed fetch waits
//!   the full refresh interval before being retried, so a flapping
//!   upstream doesn't cause a retry storm.
//! - `agents.agent_card_fetched_at` is bumped only on success (200 or
//!   304) — that's what feeds the health state machine.
//!
//! What's intentionally NOT here:
//!
//! - Smart per-row backoff (e.g. exponential). The refresh interval
//!   is global. Per-row tuning would warrant a `next_retry_at` column
//!   and a separate health/staleness audit; out of scope for v1.
//! - Sharding across replicas. The SKIP LOCKED pattern is enough at
//!   the scale we expect for v1 (hundreds of agents). Above that,
//!   shard by `kid hash(account_id)` and run job-per-shard.

use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::watch;
use tokio::time::{sleep, MissedTickBehavior};
use uuid::Uuid;

use super::fetcher::{cache_card_for_agent, CacheError, Fetcher};

/// How often the refresh loop wakes up. Each tick claims at most one
/// stale row, so with 100 push agents and 60s tick + 60min refresh
/// window the loop catches up to a clean cycle in ~100 minutes.
/// Operators with more agents should run multiple replicas — SKIP
/// LOCKED makes that safe.
pub const DEFAULT_TICK_INTERVAL_SECONDS: u64 = 60;

/// How old a row's `agent_card_last_attempted_at` must be before the
/// claim picks it up. Set to match the discovery spec's 60-minute
/// hard refresh cap.
pub const DEFAULT_STALENESS_SECONDS: u64 = 3600;

/// Outcome of a single tick of the refresh loop. Useful for tests
/// that want to drive the job step-by-step rather than spawning the
/// long-running task.
#[derive(Debug, PartialEq)]
pub enum TickOutcome {
    /// No stale push agents — nothing to do this tick.
    Idle,
    /// Claimed a row and refreshed its cache successfully.
    Refreshed { agent_id: Uuid },
    /// Claimed a row but the fetch failed. The row's
    /// `agent_card_last_attempted_at` is bumped so we won't re-claim
    /// it for one staleness window.
    FetchFailed {
        agent_id: Uuid,
        error: String,
    },
}

/// Run a single tick: claim at most one stale push agent and refresh
/// it. Idempotent: safe to call concurrently from multiple replicas.
pub async fn tick(
    pool: &PgPool,
    fetcher: &Fetcher,
    relay_base_url: &str,
    staleness_seconds: u64,
) -> TickOutcome {
    let claim = sqlx::query!(
        r#"
        WITH candidate AS (
            SELECT id
              FROM agents
             WHERE mode = 'push'
               AND tombstoned_at IS NULL
               AND (agent_card_last_attempted_at IS NULL
                    OR agent_card_last_attempted_at <
                        now() - make_interval(secs => $1::double precision))
             ORDER BY agent_card_last_attempted_at NULLS FIRST
             LIMIT 1
             FOR UPDATE SKIP LOCKED
        )
        UPDATE agents
           SET agent_card_last_attempted_at = now()
         WHERE id = (SELECT id FROM candidate)
         RETURNING id
        "#,
        staleness_seconds as f64,
    )
    .fetch_optional(pool)
    .await;

    let agent_id = match claim {
        Ok(Some(row)) => row.id,
        Ok(None) => return TickOutcome::Idle,
        Err(e) => {
            tracing::error!(error = %e, "refresh job claim query failed");
            return TickOutcome::Idle;
        }
    };

    match cache_card_for_agent(pool, fetcher, agent_id, relay_base_url).await {
        Ok(_) => TickOutcome::Refreshed { agent_id },
        Err(CacheError::NotFound | CacheError::NotPushMode) => {
            // Row changed underneath us between claim and cache —
            // benign. Treat as idle for telemetry.
            TickOutcome::Idle
        }
        Err(e) => TickOutcome::FetchFailed {
            agent_id,
            error: e.to_string(),
        },
    }
}

/// Spawn the refresh loop. Returns a watch channel sender used to
/// signal shutdown. The task exits cleanly when the receiver sees
/// `true`.
pub fn spawn(
    pool: PgPool,
    relay_base_url: String,
    tick_interval_seconds: u64,
    staleness_seconds: u64,
) -> watch::Sender<bool> {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        let fetcher = Fetcher::new();
        let mut interval = tokio::time::interval(Duration::from_secs(tick_interval_seconds));
        // If a tick is delayed by a long handler, don't try to "catch
        // up" — just resume the cadence from now.
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match tick(&pool, &fetcher, &relay_base_url, staleness_seconds).await {
                        TickOutcome::Idle => {}
                        TickOutcome::Refreshed { agent_id } => {
                            tracing::info!(agent_id = %agent_id, "card refreshed");
                        }
                        TickOutcome::FetchFailed { agent_id, error } => {
                            tracing::warn!(
                                agent_id = %agent_id,
                                error = %error,
                                "card refresh failed; will retry after staleness window",
                            );
                        }
                    }
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() {
                        tracing::info!("refresh job shutting down");
                        break;
                    }
                }
            }
        }
    });

    shutdown_tx
}

/// Sleep some, then return — useful for tests that need to coax tokio
/// timers without coupling to wall-clock fairness.
#[allow(dead_code)]
pub(crate) async fn yield_for_testing() {
    sleep(Duration::from_millis(10)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Spin up a tiny upstream and return its agent-card URL. Counts
    /// requests so tests can assert how many times the refresh job
    /// hit it.
    struct CountingUpstream {
        url: String,
        hits: Arc<AtomicUsize>,
    }

    async fn start_counting_upstream(body: serde_json::Value) -> CountingUpstream {
        let body = Arc::new(body.to_string());
        let hits = Arc::new(AtomicUsize::new(0));

        #[derive(Clone)]
        struct State {
            body: Arc<String>,
            hits: Arc<AtomicUsize>,
        }
        async fn handler(
            axum::extract::State(s): axum::extract::State<State>,
        ) -> axum::response::Response {
            s.hits.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, [("content-type", "application/json")], (*s.body).clone())
                .into_response()
        }
        let app = axum::Router::new()
            .route("/.well-known/agent-card.json", get(handler))
            .with_state(State {
                body,
                hits: hits.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        CountingUpstream {
            url: format!("http://{}/.well-known/agent-card.json", addr),
            hits,
        }
    }

    fn sample_card() -> serde_json::Value {
        serde_json::json!({
            "name": "Push Agent",
            "description": "An A2A push agent.",
            "supported_interfaces": [{
                "url": "https://upstream/a2a",
                "protocol_binding": "JSONRPC",
                "protocol_version": "0.3",
            }],
            "version": "1.0.0",
            "capabilities": { "streaming": false },
            "default_input_modes": ["application/json"],
            "default_output_modes": ["application/json"],
            "skills": [],
        })
    }

    async fn seed_account(pool: &PgPool, slug: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Acme', 'individual')"#,
            id,
            slug,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_push_agent(
        pool: &PgPool,
        account_id: Uuid,
        slug: &str,
        upstream_url: &str,
    ) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, mode, agent_card_url)
               VALUES ($1, $2, $3, 'Push', 'push', $4)"#,
            id,
            account_id,
            slug,
            upstream_url,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_pull_agent(pool: &PgPool, account_id: Uuid, slug: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name)
               VALUES ($1, $2, $3, 'Pull')"#,
            id,
            account_id,
            slug,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_idle_when_no_push_agents(pool: PgPool) {
        let acct = seed_account(&pool, "acme").await;
        seed_pull_agent(&pool, acct, "puller").await;
        let f = Fetcher::new();
        assert_eq!(
            tick(&pool, &f, "https://r", DEFAULT_STALENESS_SECONDS).await,
            TickOutcome::Idle
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_picks_a_push_agent_with_no_prior_attempt(pool: PgPool) {
        let upstream = start_counting_upstream(sample_card()).await;
        let acct = seed_account(&pool, "acme").await;
        let agent_id = seed_push_agent(&pool, acct, "pusher", &upstream.url).await;

        let f = Fetcher::new();
        let outcome = tick(&pool, &f, "https://chakramcp.com", DEFAULT_STALENESS_SECONDS).await;
        assert_eq!(outcome, TickOutcome::Refreshed { agent_id });
        assert_eq!(upstream.hits.load(Ordering::SeqCst), 1);

        // last_attempted_at and fetched_at both bumped.
        let row = sqlx::query!(
            r#"SELECT agent_card_fetched_at, agent_card_last_attempted_at,
                      agent_card_cached IS NOT NULL AS "cached!"
                 FROM agents WHERE id = $1"#,
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.agent_card_fetched_at.is_some());
        assert!(row.agent_card_last_attempted_at.is_some());
        assert!(row.cached);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_skips_recently_attempted_rows(pool: PgPool) {
        let upstream = start_counting_upstream(sample_card()).await;
        let acct = seed_account(&pool, "acme").await;
        let agent_id = seed_push_agent(&pool, acct, "pusher", &upstream.url).await;

        // Seed last_attempted_at to "just now" so the staleness gate
        // rejects this row.
        sqlx::query!(
            "UPDATE agents SET agent_card_last_attempted_at = now() WHERE id = $1",
            agent_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let f = Fetcher::new();
        // Use a 60s staleness window — last_attempted_at is < 1s ago,
        // so the row is too fresh.
        let outcome = tick(&pool, &f, "https://r", 60).await;
        assert_eq!(outcome, TickOutcome::Idle);
        assert_eq!(upstream.hits.load(Ordering::SeqCst), 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_picks_oldest_attempt_first(pool: PgPool) {
        let upstream_a = start_counting_upstream(sample_card()).await;
        let upstream_b = start_counting_upstream(sample_card()).await;
        let acct = seed_account(&pool, "acme").await;

        let agent_a = seed_push_agent(&pool, acct, "older", &upstream_a.url).await;
        let agent_b = seed_push_agent(&pool, acct, "newer", &upstream_b.url).await;

        // Both agents start with NULL last_attempted_at. Set agent_b's
        // to "1 hour ago" and agent_a's to "2 hours ago" so order is
        // deterministic (NULLS FIRST would otherwise pick arbitrarily).
        sqlx::query!(
            "UPDATE agents SET agent_card_last_attempted_at = now() - interval '2 hours' WHERE id = $1",
            agent_a,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "UPDATE agents SET agent_card_last_attempted_at = now() - interval '1 hour' WHERE id = $1",
            agent_b,
        )
        .execute(&pool)
        .await
        .unwrap();

        let f = Fetcher::new();
        let staleness = 60; // 60 seconds — both rows are stale relative to this
        let first = tick(&pool, &f, "https://r", staleness).await;
        assert_eq!(first, TickOutcome::Refreshed { agent_id: agent_a });

        let second = tick(&pool, &f, "https://r", staleness).await;
        assert_eq!(second, TickOutcome::Refreshed { agent_id: agent_b });

        let third = tick(&pool, &f, "https://r", staleness).await;
        assert_eq!(third, TickOutcome::Idle);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_records_failure_without_breaking_loop(pool: PgPool) {
        // Point at a port nothing is listening on — fetch fails fast.
        let unreachable = "http://127.0.0.1:1/.well-known/agent-card.json";
        let acct = seed_account(&pool, "acme").await;
        let agent_id = seed_push_agent(&pool, acct, "broken", unreachable).await;

        let f = Fetcher::new();
        let outcome = tick(&pool, &f, "https://r", DEFAULT_STALENESS_SECONDS).await;
        match outcome {
            TickOutcome::FetchFailed { agent_id: returned, .. } => assert_eq!(returned, agent_id),
            other => panic!("expected FetchFailed, got {other:?}"),
        }

        // last_attempted_at IS bumped (claim happened) but
        // fetched_at is NOT (success-only).
        let row = sqlx::query!(
            r#"SELECT agent_card_fetched_at, agent_card_last_attempted_at
                 FROM agents WHERE id = $1"#,
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.agent_card_last_attempted_at.is_some());
        assert!(row.agent_card_fetched_at.is_none());

        // Second tick within the staleness window does NOT retry —
        // backoff via attempt-time bump is working.
        let again = tick(&pool, &f, "https://r", DEFAULT_STALENESS_SECONDS).await;
        assert_eq!(again, TickOutcome::Idle);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_skips_tombstoned_agents(pool: PgPool) {
        let upstream = start_counting_upstream(sample_card()).await;
        let acct = seed_account(&pool, "acme").await;
        let agent_id = seed_push_agent(&pool, acct, "tombed", &upstream.url).await;
        sqlx::query!("UPDATE agents SET tombstoned_at = now() WHERE id = $1", agent_id)
            .execute(&pool)
            .await
            .unwrap();

        let f = Fetcher::new();
        let outcome = tick(&pool, &f, "https://r", DEFAULT_STALENESS_SECONDS).await;
        assert_eq!(outcome, TickOutcome::Idle);
        assert_eq!(upstream.hits.load(Ordering::SeqCst), 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tick_skips_pull_agents(pool: PgPool) {
        let acct = seed_account(&pool, "acme").await;
        let _pull = seed_pull_agent(&pool, acct, "puller").await;
        let f = Fetcher::new();
        let outcome = tick(&pool, &f, "https://r", DEFAULT_STALENESS_SECONDS).await;
        assert_eq!(outcome, TickOutcome::Idle);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn concurrent_ticks_pick_different_rows(pool: PgPool) {
        // Two ticks fired in parallel must not both pick the same row
        // (SKIP LOCKED). With two stale rows and two concurrent ticks,
        // each must claim a distinct agent.
        let up_a = start_counting_upstream(sample_card()).await;
        let up_b = start_counting_upstream(sample_card()).await;
        let acct = seed_account(&pool, "acme").await;
        let a = seed_push_agent(&pool, acct, "a", &up_a.url).await;
        let b = seed_push_agent(&pool, acct, "b", &up_b.url).await;

        let f1 = Fetcher::new();
        let f2 = Fetcher::new();
        let pool1 = pool.clone();
        let pool2 = pool.clone();
        let staleness = DEFAULT_STALENESS_SECONDS;
        let (r1, r2) = tokio::join!(
            tick(&pool1, &f1, "https://r", staleness),
            tick(&pool2, &f2, "https://r", staleness)
        );

        // Each tick must have picked a distinct agent (or one picked
        // nothing if the SKIP LOCKED races; both picking the same row
        // is the failure mode this test guards against).
        match (r1, r2) {
            (TickOutcome::Refreshed { agent_id: x }, TickOutcome::Refreshed { agent_id: y }) => {
                assert_ne!(x, y, "two ticks must not pick the same row");
                let claimed = [x, y];
                assert!(claimed.contains(&a));
                assert!(claimed.contains(&b));
            }
            // One claimed, the other was idle — also fine; SKIP LOCKED
            // races sometimes drop a candidate. Just assert no double-claim.
            (TickOutcome::Refreshed { agent_id }, TickOutcome::Idle)
            | (TickOutcome::Idle, TickOutcome::Refreshed { agent_id }) => {
                assert!(agent_id == a || agent_id == b);
            }
            other => panic!("unexpected concurrent outcome: {other:?}"),
        }
    }
}
