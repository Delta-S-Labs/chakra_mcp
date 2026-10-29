//! Claims and deadlines end to end (waking pull agents §9, §10, W4): leases,
//! release, late answers, `expires_in`, and the sweep.

use std::time::Duration;

use axum::http::StatusCode;
use chakramcp_shared::wake::{Wake, WakeSettings};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::calls::tests::{calls_on, get, grant, mcp, post, world, World};
use crate::invocation_sweep::{sweep_once, Swept};
use crate::state::RelayState;

fn relay(pool: &PgPool) -> RelayState {
    calls_on(pool).with_wake(
        Wake::new(10),
        WakeSettings {
            max_wait_secs: 10,
            max_waiters: 10,
        },
    )
}

/// Alice's agent calls Bob's through `grant_id`, with `extra` in the body.
async fn invoke(
    state: &RelayState,
    w: &World,
    grant_id: Uuid,
    extra: Value,
) -> (StatusCode, Value) {
    let mut body =
        json!({ "grant_id": grant_id, "grantee_agent_id": w.a, "input": { "venue": "Hall A" } });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    post(state, &w.alice.key, "/v1/invoke", body).await
}

async fn queued(state: &RelayState, w: &World, grant_id: Uuid) -> Uuid {
    let (status, body) = invoke(state, w, grant_id, json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body["invocation_id"].as_str().unwrap().parse().unwrap()
}

/// Bob's agent claims from its inbox with `query` (e.g. `&lease_secs=45`).
async fn claim(state: &RelayState, w: &World, query: &str) -> Value {
    let (status, body) = get(
        state,
        &w.bob.key,
        &format!("/v1/inbox?agent_id={}{query}", w.b),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[derive(Debug)]
struct Row {
    status: String,
    deliveries: i32,
    /// Seconds from now to `lease_until`.
    lease: Option<i64>,
    /// Seconds from `created_at` to `expires_at`.
    ttl: Option<i64>,
    claimed: bool,
    error: Option<String>,
}

async fn row(pool: &PgPool, id: Uuid) -> Row {
    let r: (String, i32, Option<i64>, Option<i64>, bool, Option<String>) = sqlx::query_as(
        "SELECT status, deliveries,
                EXTRACT(EPOCH FROM lease_until - now())::bigint,
                EXTRACT(EPOCH FROM expires_at - created_at)::bigint,
                claimed_at IS NOT NULL, error_message
           FROM relay_invocations WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    Row {
        status: r.0,
        deliveries: r.1,
        lease: r.2,
        ttl: r.3,
        claimed: r.4,
        error: r.5,
    }
}

async fn exec(pool: &PgPool, sql: &'static str, id: Uuid) {
    sqlx::query(sql).bind(id).execute(pool).await.unwrap();
}

async fn human_in_loop(pool: &PgPool, agent: Uuid) {
    exec(
        pool,
        "UPDATE agent_capabilities SET semantics = 'human_in_loop' WHERE agent_id = $1",
        agent,
    )
    .await;
}

async fn sweep(state: &RelayState) -> Swept {
    sweep_once(&state.db, &state.wake, &state.invocations)
        .await
        .unwrap()
}

fn code(body: &Value) -> &str {
    body["error"]["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../migrations")]
async fn a_claim_counts_a_delivery_and_holds_for_a_lease(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let first = queued(&state, &w, g).await;
    let second = queued(&state, &w, g).await;

    let claimed = claim(&state, &w, "&limit=1&lease_secs=45").await;
    assert_eq!(claimed[0]["id"], first.to_string());
    assert_eq!(claimed[0]["deliveries"], 1);
    assert!(claimed[0]["lease_until"].is_string(), "{claimed}");
    assert!(claimed[0]["expires_at"].is_string(), "{claimed}");
    let r = row(&pool, first).await;
    assert_eq!((r.status.as_str(), r.deliveries), ("in_progress", 1));
    assert!((40..=45).contains(&r.lease.unwrap()), "{r:?}");
    assert_eq!(r.ttl, Some(86_400), "the default deadline");

    // No lease asked for: the server's 10 minutes; a huge one is a day.
    claim(&state, &w, "&limit=1").await;
    assert!((590..=600).contains(&row(&pool, second).await.lease.unwrap()));
    let third = queued(&state, &w, g).await;
    claim(&state, &w, "&limit=1&lease_secs=999999").await;
    assert!(row(&pool, third).await.lease.unwrap() > 86_000);

    // `human_in_loop` claims have no lease: their owner answers.
    human_in_loop(&pool, w.b).await;
    let asked = queued(&state, &w, g).await;
    assert_eq!(row(&pool, asked).await.ttl, Some(7 * 86_400));
    claim(&state, &w, "&limit=1").await;
    let r = row(&pool, asked).await;
    assert_eq!((r.status.as_str(), r.lease), ("in_progress", None));
}

#[sqlx::test(migrations = "../migrations")]
async fn release_hands_a_claim_back(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let id = queued(&state, &w, g).await;
    let release = format!("/v1/invocations/{id}/release");

    // Not claimed yet.
    let (status, body) = post(&state, &w.bob.key, &release, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "chk.invocation.not_releasable");

    claim(&state, &w, "").await;
    // The caller's side can't release it.
    let (status, _) = post(&state, &w.alice.key, &release, json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A pull waiting meanwhile gets it straight back.
    let waiting = {
        let (state, key, b) = (state.clone(), w.bob.key.clone(), w.b);
        tokio::spawn(
            async move { get(&state, &key, &format!("/v1/inbox?agent_id={b}&wait=5")).await },
        )
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (status, body) = post(&state, &w.bob.key, &release, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "pending");
    assert_eq!(body["deliveries"], 0, "a hand-back isn't a delivery");
    assert!(body["lease_until"].is_null());
    let (_, pulled) = tokio::time::timeout(Duration::from_secs(3), waiting)
        .await
        .expect("the waiting pull was woken")
        .unwrap();
    assert_eq!(pulled[0]["id"], id.to_string(), "{pulled}");
    assert_eq!(row(&pool, id).await.deliveries, 1);

    // Answered: settled.
    let (status, _) = post(
        &state,
        &w.bob.key,
        &format!("/v1/invocations/{id}/result"),
        json!({ "status": "succeeded", "output": { "ok": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(&state, &w.bob.key, &release, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&body), "chk.invocation.already_settled");
}

#[sqlx::test(migrations = "../migrations")]
async fn a_reply_waiting_for_the_owner_isnt_released(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    human_in_loop(&pool, w.b).await;
    let state = relay(&pool);
    let id = queued(&state, &w, g).await;
    claim(&state, &w, "").await;
    // An agent's reply to a human_in_loop call is a draft for the owner.
    let (status, body) = post(
        &state,
        &w.bob.key,
        &format!("/v1/invocations/{id}/result"),
        json!({ "status": "succeeded", "output": { "ok": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, body) = post(
        &state,
        &w.bob.key,
        &format!("/v1/invocations/{id}/release"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "chk.invocation.not_releasable");
}

#[sqlx::test(migrations = "../migrations")]
async fn a_result_needs_a_claim_and_the_first_answer_wins(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let id = queued(&state, &w, g).await;
    let result = format!("/v1/invocations/{id}/result");
    let answer = json!({ "status": "succeeded", "output": { "ok": true } });

    let (status, body) = post(&state, &w.bob.key, &result, answer.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "chk.invocation.not_claimed");

    // Claimed, and its lease runs out: back to pending for another pull.
    claim(&state, &w, "").await;
    exec(
        &pool,
        "UPDATE relay_invocations SET lease_until = now() - interval '1 second' WHERE id = $1",
        id,
    )
    .await;
    assert_eq!(sweep(&state).await.pended, 1);
    let r = row(&pool, id).await;
    assert_eq!(
        (r.status.as_str(), r.deliveries, r.claimed),
        ("pending", 1, false)
    );

    // The first worker finishes late: its answer still lands.
    let (status, body) = post(&state, &w.bob.key, &result, answer.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "succeeded");
    let (status, body) = post(&state, &w.bob.key, &result, answer).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "chk.invocation.already_settled");
}

#[sqlx::test(migrations = "../migrations")]
async fn a_lease_runs_out_three_times_at_most(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let id = queued(&state, &w, g).await;
    for round in 1..=3 {
        assert_eq!(claim(&state, &w, "").await.as_array().unwrap().len(), 1);
        exec(
            &pool,
            "UPDATE relay_invocations SET lease_until = now() - interval '1 second' WHERE id = $1",
            id,
        )
        .await;
        let swept = sweep(&state).await;
        if round < 3 {
            assert_eq!(swept.pended, 1, "round {round}");
        } else {
            assert_eq!((swept.pended, swept.lease_exhausted), (0, 1));
        }
    }
    let r = row(&pool, id).await;
    assert_eq!(r.status, "timeout");
    assert_eq!(r.error.as_deref(), Some("claimed 3 times, never answered"));
    let (_, notices) = get(
        &state,
        &w.bob.key,
        &format!("/v1/inbox/notices?agent_id={}", w.b),
    )
    .await;
    assert_eq!(notices[0]["kind"], "invocation_expired", "{notices}");
    assert_eq!(notices[0]["invocation_id"], id.to_string());
}

#[sqlx::test(migrations = "../migrations")]
async fn expires_in_is_checked_and_clamped(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let (status, body) = invoke(&state, &w, g, json!({ "expires_in": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM relay_invocations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "refused before anything is recorded");

    let (_, body) = invoke(&state, &w, g, json!({ "expires_in": 60 })).await;
    let short = body["invocation_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(row(&pool, short).await.ttl, Some(60));
    let (_, body) = invoke(&state, &w, g, json!({ "expires_in": 30 * 86_400 })).await;
    let long = body["invocation_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        row(&pool, long).await.ttl,
        Some(86_400),
        "clamped to the default"
    );

    // MCP's invoke takes it too.
    let result = mcp(
        &state,
        &w.alice.key,
        "tools/call",
        json!({ "name": "invoke", "arguments": {
            "grant_id": g, "grantee_agent_id": w.a, "input": {}, "expires_in": 120 } }),
    )
    .await;
    let id = result["structuredContent"]["invocation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(row(&pool, id).await.ttl, Some(120));
}

#[sqlx::test(migrations = "../migrations")]
async fn approval_starts_the_deadline_again(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    exec(
        &pool,
        "UPDATE agent_capabilities SET consent_required = true WHERE agent_id = $1",
        w.b,
    )
    .await;
    let state = relay(&pool);
    let (status, held) = invoke(&state, &w, g, json!({ "expires_in": 600 })).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{held}");
    assert_eq!(held["status"], "awaiting_consent");
    let id: Uuid = held["invocation_id"].as_str().unwrap().parse().unwrap();
    // It waited an hour for its owner.
    exec(
        &pool,
        "UPDATE relay_invocations SET created_at = created_at - interval '1 hour',
                expires_at = expires_at - interval '1 hour' WHERE id = $1",
        id,
    )
    .await;
    let request = held["consent_request_id"].as_str().unwrap();
    let (status, body) = post(
        &state,
        &w.bob.web,
        &format!("/v1/approvals/{request}/approve"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let left: i64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM expires_at - now())::bigint FROM relay_invocations WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        (590..=600).contains(&left),
        "{left}: ten minutes from approval"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn the_sweep_gives_old_calls_a_deadline(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let ask = grant(&pool, w.b, w.a, "ask_owner").await;
    sqlx::query(
        "UPDATE agent_capabilities SET semantics = 'human_in_loop' WHERE agent_id = $1 AND name = 'ask_owner'",
    )
    .bind(w.b)
    .execute(&pool)
    .await
    .unwrap();
    let state = relay(&pool);
    let plain = queued(&state, &w, g).await;
    let asked = queued(&state, &w, ask).await;
    exec(
        &pool,
        "UPDATE relay_invocations SET expires_at = NULL, created_at = created_at - interval '3 days'
          WHERE id = $1",
        plain,
    )
    .await;
    exec(
        &pool,
        "UPDATE relay_invocations SET expires_at = NULL WHERE id = $1",
        asked,
    )
    .await;
    assert_eq!(sweep(&state).await.backfilled, 2);
    let left = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT EXTRACT(EPOCH FROM expires_at - now())::bigint FROM relay_invocations WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert!(
        (86_390..=86_400).contains(&left(plain).await),
        "a full day from now"
    );
    assert!(left(asked).await > 7 * 86_400 - 10);
    assert_eq!(row(&pool, plain).await.status, "pending", "not timed out");
    assert_eq!(sweep(&state).await, Swept::default(), "nothing left");
}

#[sqlx::test(migrations = "../migrations")]
async fn calls_past_their_deadline_time_out(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let claimed = queued(&state, &w, g).await;
    let unclaimed = queued(&state, &w, g).await;
    claim(&state, &w, "&limit=1").await; // the older one
    for id in [claimed, unclaimed] {
        exec(
            &pool,
            "UPDATE relay_invocations SET expires_at = now() - interval '1 second' WHERE id = $1",
            id,
        )
        .await;
    }
    let swept = sweep(&state).await;
    assert_eq!(
        (swept.not_answered, swept.not_picked_up),
        (1, 1),
        "{swept:?}"
    );
    let r = row(&pool, claimed).await;
    assert_eq!(r.status, "timeout");
    assert_eq!(r.error.as_deref(), Some("picked up, never answered"));
    let r = row(&pool, unclaimed).await;
    assert_eq!(r.status, "timeout");
    assert_eq!(
        r.error.as_deref(),
        Some("not picked up before its deadline")
    );

    // The called agent hears about both, and a late answer is refused.
    let (_, notices) = get(
        &state,
        &w.bob.key,
        &format!("/v1/inbox/notices?agent_id={}", w.b),
    )
    .await;
    let mut told: Vec<&str> = notices
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "invocation_expired")
        .map(|n| n["invocation_id"].as_str().unwrap())
        .collect();
    told.sort();
    let mut want = [claimed.to_string(), unclaimed.to_string()];
    want.sort();
    assert_eq!(told, want);
    let (status, body) = post(
        &state,
        &w.bob.key,
        &format!("/v1/invocations/{claimed}/result"),
        json!({ "status": "succeeded", "output": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&body), "chk.invocation.already_settled");
    // The caller sees why.
    let (_, seen) = get(
        &state,
        &w.alice.key,
        &format!("/v1/invocations/{unclaimed}"),
    )
    .await;
    assert_eq!(seen["status"], "timeout");
    assert_eq!(seen["error_message"], "not picked up before its deadline");
}

#[sqlx::test(migrations = "../migrations")]
async fn a_timed_out_call_cancels_its_draft_unless_someone_holds_it(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    human_in_loop(&pool, w.b).await;
    let state = relay(&pool);
    let id = queued(&state, &w, g).await;
    claim(&state, &w, "").await;
    let (status, _) = post(
        &state,
        &w.bob.key,
        &format!("/v1/invocations/{id}/result"),
        json!({ "status": "succeeded", "output": { "ok": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "a draft");
    exec(
        &pool,
        "UPDATE relay_invocations SET expires_at = now() - interval '1 second' WHERE id = $1",
        id,
    )
    .await;

    // An approver is deciding on the draft: the sweep leaves the call.
    let mut deciding = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM consent_requests WHERE invocation_id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *deciding)
        .await
        .unwrap();
    assert_eq!(sweep(&state).await.not_answered, 0);
    assert_eq!(row(&pool, id).await.status, "in_progress");
    deciding.rollback().await.unwrap();

    assert_eq!(sweep(&state).await.not_answered, 1);
    assert_eq!(row(&pool, id).await.status, "timeout");
    let draft: String = sqlx::query_scalar(
        "SELECT status FROM consent_requests WHERE invocation_id = $1 AND kind = 'reply'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(draft, "cancelled");
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_releases_and_claims_with_a_lease(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    let id = queued(&state, &w, g).await;
    let pulled = mcp(
        &state,
        &w.bob.key,
        "tools/call",
        json!({ "name": "pull_inbox", "arguments": { "agent_id": w.b, "lease_secs": 90 } }),
    )
    .await;
    let items: Value =
        serde_json::from_str(pulled["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(items[0]["lease_until"].is_string(), "{items}");
    assert!((85..=90).contains(&row(&pool, id).await.lease.unwrap()));

    let released = mcp(
        &state,
        &w.bob.key,
        "tools/call",
        json!({ "name": "release_invocation", "arguments": { "invocation_id": id } }),
    )
    .await;
    assert_eq!(released["isError"], false, "{released}");
    assert_eq!(released["structuredContent"]["status"], "pending");
    let again = mcp(
        &state,
        &w.bob.key,
        "tools/call",
        json!({ "name": "release_invocation", "arguments": { "invocation_id": id } }),
    )
    .await;
    assert_eq!(again["isError"], true);
    assert_eq!(
        again["structuredContent"]["code"], "chk.invocation.not_releasable",
        "{again}"
    );
}

/// `relay_invocations` is written and read by every invocation. While 0053
/// waits for its lock, readers must not queue behind it: it tries for
/// 100 ms at a time and backs off in between. Then 0054 builds the indexes.
#[sqlx::test(migrations = false)]
async fn the_lease_columns_never_park_readers_behind_their_lock(pool: PgPool) {
    use std::time::Instant;

    const BEFORE: i64 = 52;
    const COLUMNS: i64 = 53;
    let migrator = sqlx::migrate!("../migrations");
    migrator.run_to(BEFORE, &pool).await.unwrap();
    let wide = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();

    // A long read of the table is in flight when the migration starts...
    let mut long_read = wide.begin().await.unwrap();
    sqlx::query("SELECT count(*) FROM relay_invocations")
        .execute(&mut *long_read)
        .await
        .unwrap();
    let migration = tokio::spawn({
        let wide = wide.clone();
        async move { sqlx::migrate!("../migrations").run_to(COLUMNS, &wide).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ...and a new read arriving meanwhile waits at most one short attempt.
    let mut reader = wide.acquire().await.unwrap();
    sqlx::query("SET lock_timeout = '1s'")
        .execute(&mut *reader)
        .await
        .unwrap();
    for _ in 0..3 {
        let started = Instant::now();
        sqlx::query("SELECT count(*) FROM relay_invocations")
            .execute(&mut *reader)
            .await
            .expect("a reader was parked behind the migration");
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "a reader waited {:?} behind the migration",
            started.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(reader);
    assert!(!migration.is_finished(), "still waiting for the long read");

    tokio::time::sleep(Duration::from_millis(500)).await;
    long_read.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), migration)
        .await
        .expect("retries until the lock is free")
        .unwrap()
        .unwrap();
    migrator.run(&pool).await.unwrap();
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname::text FROM pg_indexes
          WHERE tablename = 'relay_invocations'
            AND indexname IN ('relay_invocations_open_expiry', 'relay_invocations_live_lease')
          ORDER BY indexname",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        indexes,
        [
            "relay_invocations_live_lease",
            "relay_invocations_open_expiry"
        ]
    );
}
