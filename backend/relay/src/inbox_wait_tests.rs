//! Waiting pulls end to end (waking pull agents §6, W1): each feed wakes
//! when work is queued for it, and a pull returns at its deadline, past the
//! waiter cap, and at shutdown.

use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chakramcp_shared::wake::{Wake, WakeSettings};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::calls::tests::{active, calls_on, grant, post, ring, say, tool, world, World};
use crate::state::RelayState;

/// A relay that waits at most `max_wait` seconds, `max_waiters` at a time.
fn relay(pool: &PgPool, max_wait: u32, max_waiters: usize) -> RelayState {
    calls_on(pool).with_wake(
        Wake::new(max_waiters),
        WakeSettings {
            max_wait_secs: max_wait,
            max_waiters,
        },
    )
}

struct Pulled {
    status: StatusCode,
    waited: Option<String>,
    body: Value,
    took: Duration,
}

impl Pulled {
    fn len(&self) -> usize {
        self.body.as_array().map_or(0, Vec::len)
    }
}

async fn pull(state: &RelayState, token: &str, uri: &str) -> Pulled {
    let started = Instant::now();
    let res = crate::router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let waited = res
        .headers()
        .get("chakramcp-wait")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Pulled {
        status,
        waited,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        took: started.elapsed(),
    }
}

/// Bob's agent's invocation inbox, waiting `wait` seconds.
fn inbox(w: &World, wait: u32) -> String {
    format!("/v1/inbox?agent_id={}&wait={wait}", w.b)
}

/// Start a pull in the background, and give it time to start waiting.
async fn pulling(state: &RelayState, token: &str, uri: String) -> tokio::task::JoinHandle<Pulled> {
    let (state, token) = (state.clone(), token.to_owned());
    let handle = tokio::spawn(async move { pull(&state, &token, &uri).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle
}

/// Alice's agent calls Bob's.
async fn invoke(state: &RelayState, w: &World, grant_id: Uuid) -> Value {
    let (status, body) = post(
        state,
        &w.alice.key,
        "/v1/invoke",
        json!({ "grant_id": grant_id, "grantee_agent_id": w.a, "input": { "venue": "Hall A" } }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body
}

#[sqlx::test(migrations = "../migrations")]
async fn a_pull_without_wait_answers_at_once(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool, 25, 10);
    let p = pull(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    assert_eq!(p.status, StatusCode::OK);
    assert_eq!(p.waited.as_deref(), Some("0"));
    assert_eq!(p.len(), 0);
    assert!(p.took < Duration::from_secs(1));
}

#[sqlx::test(migrations = "../migrations")]
async fn a_waiting_pull_gets_an_invocation_queued_while_it_waits(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool, 25, 10);
    let waiting = pulling(&state, &w.bob.key, inbox(&w, 10)).await;
    invoke(&state, &w, g).await;
    let p = waiting.await.unwrap();
    assert_eq!(p.status, StatusCode::OK, "{}", p.body);
    assert_eq!(p.waited.as_deref(), Some("10"));
    assert_eq!(p.len(), 1, "{}", p.body);
    assert_eq!(p.body[0]["status"], "in_progress");
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);
}

#[sqlx::test(migrations = "../migrations")]
async fn a_waiting_pull_returns_empty_at_its_deadline_and_is_clamped(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool, 1, 10);
    let p = pull(&state, &w.bob.key, &inbox(&w, 600)).await;
    assert_eq!(p.status, StatusCode::OK);
    assert_eq!(p.waited.as_deref(), Some("1"));
    assert_eq!(p.len(), 0);
    assert!(p.took >= Duration::from_millis(900), "{:?}", p.took);
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);
}

#[sqlx::test(migrations = "../migrations")]
async fn two_waiting_pullers_and_one_invocation(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool, 2, 10);
    let first = pulling(&state, &w.bob.key, inbox(&w, 2)).await;
    let second = pulling(&state, &w.bob.key, inbox(&w, 2)).await;
    invoke(&state, &w, g).await;
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    let mut got = [first.len(), second.len()];
    got.sort();
    assert_eq!(got, [0, 1]);
}

#[sqlx::test(migrations = "../migrations")]
async fn past_the_waiter_cap_a_pull_answers_at_once(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool, 25, 1);
    let first = pulling(&state, &w.bob.key, inbox(&w, 2)).await;
    let second = pull(&state, &w.bob.key, &inbox(&w, 2)).await;
    assert_eq!(second.waited.as_deref(), Some("0"));
    assert!(second.took < Duration::from_secs(1), "{:?}", second.took);
    assert_eq!(first.await.unwrap().waited.as_deref(), Some("2"));
}

#[sqlx::test(migrations = "../migrations")]
async fn shutdown_releases_a_waiting_pull(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool, 25, 10);
    let waiting = pulling(&state, &w.bob.key, inbox(&w, 20)).await;
    state.wake.close();
    let p = waiting.await.unwrap();
    assert_eq!(p.waited.as_deref(), Some("0"));
    assert_eq!(p.len(), 0);
    assert!(p.took < Duration::from_secs(2), "{:?}", p.took);
    // New pulls don't wait either.
    let p = pull(&state, &w.bob.key, &inbox(&w, 20)).await;
    assert_eq!(p.waited.as_deref(), Some("0"));
}

#[sqlx::test(migrations = "../migrations")]
async fn a_consent_request_wakes_notices_and_its_approval_wakes_the_inbox(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    sqlx::query("UPDATE agent_capabilities SET consent_required = true WHERE agent_id = $1")
        .bind(w.b)
        .execute(&pool)
        .await
        .unwrap();
    let state = relay(&pool, 25, 10);

    let notices = pulling(
        &state,
        &w.bob.key,
        format!("/v1/inbox/notices?agent_id={}&wait=10", w.b),
    )
    .await;
    let held = invoke(&state, &w, g).await;
    let p = notices.await.unwrap();
    assert_eq!(p.len(), 1, "{}", p.body);
    assert_eq!(p.body[0]["kind"], "consent_requested");
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);

    let inbox_pull = pulling(&state, &w.bob.key, inbox(&w, 10)).await;
    let request = held["consent_request_id"].as_str().unwrap();
    let (status, body) = post(
        &state,
        &w.bob.web,
        &format!("/v1/approvals/{request}/approve"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let p = inbox_pull.await.unwrap();
    assert_eq!(p.len(), 1, "{}", p.body);
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_pull_inbox_waits_for_work(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool, 25, 10);
    let (st, key, b) = (state.clone(), w.bob.key.clone(), w.b);
    let waiting = tokio::spawn(async move {
        let started = Instant::now();
        let items = tool(
            &st,
            &key,
            "pull_inbox",
            json!({ "agent_id": b, "wait_seconds": 10 }),
        )
        .await;
        (items, started.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    invoke(&state, &w, g).await;
    let (items, took) = waiting.await.unwrap();
    assert_eq!(items.as_array().unwrap().len(), 1, "{items}");
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[sqlx::test(migrations = "../migrations")]
async fn call_events_wake_the_calls_feed(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool, 25, 10);
    let calls = |agent: Uuid| format!("/v1/inbox/calls?agent_id={agent}&wait=10");

    // An invite wakes the callee.
    let bob = pulling(&state, &w.bob.key, calls(w.b)).await;
    let call = ring(&state, &w, json!({})).await;
    let p = bob.await.unwrap();
    assert_eq!(p.waited.as_deref(), Some("10"));
    assert_eq!(p.body[0]["entry"]["kind"], "invite", "{}", p.body);
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);

    // Accepting wakes the caller.
    let alice = pulling(&state, &w.alice.key, calls(w.a)).await;
    let (status, body) = post(
        &state,
        &w.bob.key,
        &format!("/v1/calls/{call}/accept"),
        json!({ "agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let p = alice.await.unwrap();
    assert_eq!(p.body[0]["entry"]["kind"], "accepted", "{}", p.body);

    // A message wakes the other party.
    let bob = pulling(&state, &w.bob.key, calls(w.b)).await;
    let (status, body) = say(&state, &w.alice.key, call, w.a, "Is Hall A free?").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let p = bob.await.unwrap();
    assert_eq!(p.body[0]["entry"]["kind"], "message", "{}", p.body);
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);
}

#[sqlx::test(migrations = "../migrations")]
async fn ending_a_call_wakes_the_other_party(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool, 25, 10);
    let call = active(&state, &w).await;
    // Drain what's queued so far.
    pull(
        &state,
        &w.alice.key,
        &format!("/v1/inbox/calls?agent_id={}", w.a),
    )
    .await;
    let alice = pulling(
        &state,
        &w.alice.key,
        format!("/v1/inbox/calls?agent_id={}&wait=10", w.a),
    )
    .await;
    let (status, body) = post(
        &state,
        &w.bob.key,
        &format!("/v1/calls/{call}/end"),
        json!({ "agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let p = alice.await.unwrap();
    assert_eq!(p.body[0]["entry"]["kind"], "ended", "{}", p.body);
    assert!(p.took < Duration::from_secs(3), "{:?}", p.took);
}

#[sqlx::test(migrations = "../migrations")]
async fn waiting_pulls_hold_no_database_connection(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    // Five connections for a hundred pulls waiting two seconds: if a wait
    // held its connection, most would time out acquiring one.
    let small = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let state = relay(&small, 2, 1000);
    let pulls: Vec<_> = (0..100)
        .map(|_| {
            let (state, key, uri) = (state.clone(), w.bob.key.clone(), inbox(&w, 2));
            tokio::spawn(async move { pull(&state, &key, &uri).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(500)).await;
    invoke(&state, &w, g).await;
    let mut claimed = 0;
    for p in pulls {
        let p = p.await.unwrap();
        assert_eq!(p.status, StatusCode::OK, "{}", p.body);
        assert_eq!(p.waited.as_deref(), Some("2"));
        claimed += p.len();
    }
    assert_eq!(claimed, 1);
}
