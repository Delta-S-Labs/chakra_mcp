//! Wake URLs end to end (waking pull agents §7, W6a), against a local test
//! server: setting one, the ping and its signature, the text format, the
//! throttle across relay copies, failures, and `peer.wake`.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use chakramcp_shared::wake::{Feed, Wake, WakeSettings};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::calls::tests::{calls_on, get, grant, http, mcp, post as relay_post, world, World};
use crate::outbound::OutboundPolicy;
use crate::state::RelayState;
use crate::wake_urls::{signature, WakeUrlSettings, SIGNATURE_HEADER};

/// A relay with fast windows: pings group for 50 ms and are throttled for 1 s.
fn relay(pool: &PgPool) -> RelayState {
    relay_throttled(pool, Duration::from_secs(1))
}

fn relay_throttled(pool: &PgPool, throttle: Duration) -> RelayState {
    calls_on(pool)
        .with_wake(
            Wake::new(10),
            WakeSettings {
                max_wait_secs: 10,
                max_waiters: 10,
            },
        )
        .with_wake_urls(WakeUrlSettings {
            group: Duration::from_millis(50),
            throttle,
            ..WakeUrlSettings::default()
        })
}

/// What a test server was sent.
#[derive(Clone, Default)]
struct Hooked {
    pings: Arc<Mutex<Vec<(HeaderMap, String)>>>,
    status: Arc<AtomicU16>,
}

impl Hooked {
    fn count(&self) -> usize {
        self.pings.lock().unwrap().len()
    }

    fn last(&self) -> (HeaderMap, String) {
        self.pings.lock().unwrap().last().cloned().unwrap()
    }

    /// Wait up to `secs` for `n` pings.
    async fn wait_for(&self, n: usize, secs: u64) {
        for _ in 0..secs * 20 {
            if self.count() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{} pings, wanted {n}", self.count());
    }
}

/// A server recording what it's sent to `/hook`, answering 200.
async fn server() -> (String, Hooked) {
    let hooked = Hooked::default();
    hooked.status.store(200, Ordering::Relaxed);
    async fn hook(State(h): State<Hooked>, headers: HeaderMap, body: String) -> StatusCode {
        h.pings.lock().unwrap().push((headers, body));
        StatusCode::from_u16(h.status.load(Ordering::Relaxed)).unwrap()
    }
    let app = axum::Router::new()
        .route("/hook", post(hook))
        .with_state(hooked.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (format!("http://{addr}/hook"), hooked)
}

async fn put_wake(
    state: &RelayState,
    token: &str,
    agent: Uuid,
    body: Value,
) -> (StatusCode, Value) {
    http(
        state,
        axum::http::Method::PUT,
        token,
        &format!("/v1/agents/{agent}/wake"),
        Some(body),
    )
    .await
}

async fn invoke(state: &RelayState, w: &World, grant_id: Uuid) -> Value {
    let (status, body) = relay_post(
        state,
        &w.alice.key,
        "/v1/invoke",
        json!({ "grant_id": grant_id, "grantee_agent_id": w.a, "input": { "venue": "Hall A" } }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body
}

async fn handle(pool: &PgPool, agent: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT acc.slug || '/' || a.slug FROM agents a JOIN accounts acc ON acc.id = a.account_id WHERE a.id = $1",
    )
    .bind(agent)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../migrations")]
async fn owners_set_read_and_clear_a_wake_url(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool);
    let (url, _) = server().await;
    let path = format!("/v1/agents/{}/wake", w.b);

    let (status, set) = put_wake(&state, &w.bob.key, w.b, json!({ "url": url })).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(set["url"], url);
    assert_eq!(set["format"], "json");
    assert_eq!(set["enabled"], true);
    assert!(set["secret"].as_str().unwrap().starts_with("cmw_"));
    assert!(state.heartbeats.has_wake(w.b));

    // Read back without the secret; never by another account.
    let (status, read) = get(&state, &w.bob.key, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(read.get("secret").is_none(), "{read}");
    assert_eq!(
        get(&state, &w.alice.key, &path).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, _) = put_wake(&state, &w.alice.key, w.b, json!({ "url": url })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Setting it again makes a new secret.
    let (_, again) = put_wake(
        &state,
        &w.bob.key,
        w.b,
        json!({ "url": url, "format": "text" }),
    )
    .await;
    assert_ne!(again["secret"], set["secret"]);
    assert_eq!(again["format"], "text");
    let (status, body) = put_wake(
        &state,
        &w.bob.key,
        w.b,
        json!({ "url": url, "format": "xml" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, _) = http(&state, axum::http::Method::DELETE, &w.bob.key, &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        get(&state, &w.bob.key, &path).await.0,
        StatusCode::NOT_FOUND
    );
    assert!(!state.heartbeats.has_wake(w.b));
}

#[sqlx::test(migrations = "../migrations")]
async fn push_agents_and_guarded_urls_are_refused(pool: PgPool) {
    let w = world(&pool).await;
    // A managed server: public addresses, https only.
    let state = relay(&pool)
        .with_outbound(OutboundPolicy {
            allow_private: false,
        })
        .with_wake_urls(WakeUrlSettings {
            https_only: true,
            ..WakeUrlSettings::default()
        });
    for url in [
        "http://127.0.0.1:9/hook",
        "https://10.0.0.5/hook",
        "http://example.com/hook",
        "ftp://example.com/x",
        "not a url",
    ] {
        let (status, body) = put_wake(&state, &w.bob.key, w.b, json!({ "url": url })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {body}");
    }
    let (status, body) = put_wake(
        &state,
        &w.bob.key,
        w.b,
        json!({ "url": "https://example.com/hook" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    sqlx::query("UPDATE agents SET mode = 'push', agent_card_url = 'https://push.example/card' WHERE id = $1")
        .bind(w.b)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = put_wake(
        &state,
        &w.bob.key,
        w.b,
        json!({ "url": "https://example.com/hook" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("pull agents"));

    // Switched off on this server: refused.
    let off = relay(&pool).with_wake_urls(WakeUrlSettings {
        enabled: false,
        ..WakeUrlSettings::default()
    });
    let (status, _) = put_wake(
        &off,
        &w.alice.key,
        w.a,
        json!({ "url": "https://example.com/hook" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../migrations")]
async fn an_invoke_pings_the_wake_url_signed(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    crate::wake_urls::spawn(state.clone());
    let (url, hooked) = server().await;
    let (_, set) = put_wake(&state, &w.bob.key, w.b, json!({ "url": url })).await;
    let secret = set["secret"].as_str().unwrap().to_owned();

    let queued = invoke(&state, &w, g).await;
    assert_eq!(queued["peer"]["wake"], true, "{queued}");
    hooked.wait_for(1, 3).await;
    let (headers, body) = hooked.last();
    let ping: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(ping["type"], "chakramcp.wake");
    assert_eq!(ping["agent"], handle(&pool, w.b).await);
    assert_eq!(ping["agent_id"], w.b.to_string());
    assert_eq!(
        ping["pending"],
        json!({ "invocations": 1, "notices": 0, "calls": 0 })
    );
    // Nothing private: counts and the agent's own name only.
    assert!(
        !body.contains("Hall A") && !body.contains(&w.a.to_string()),
        "{body}"
    );

    let sig = headers.get(SIGNATURE_HEADER).unwrap().to_str().unwrap();
    let t: i64 = sig
        .strip_prefix("t=")
        .and_then(|rest| rest.split(',').next())
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        sig,
        signature(&secret, t, &body),
        "verifies with the secret"
    );
    assert!((chrono::Utc::now().timestamp() - t).abs() < 30);
    let (_, read) = get(&state, &w.bob.key, &format!("/v1/agents/{}/wake", w.b)).await;
    assert!(read["last_ok_at"].is_string(), "{read}");
}

#[sqlx::test(migrations = "../migrations")]
async fn text_pings_and_nothing_pending_sends_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = relay(&pool);
    crate::wake_urls::spawn(state.clone());
    let (url, hooked) = server().await;
    put_wake(
        &state,
        &w.bob.key,
        w.b,
        json!({ "url": url, "format": "text" }),
    )
    .await;

    // A wake with nothing waiting: no ping.
    state.wake.notify(w.b, Feed::Notices);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(hooked.count(), 0);

    invoke(&state, &w, g).await;
    hooked.wait_for(1, 3).await;
    let (headers, body) = hooked.last();
    assert_eq!(
        body,
        format!("{} has 1 new item on ChakraMCP", handle(&pool, w.b).await)
    );
    assert!(headers.get(SIGNATURE_HEADER).is_none());
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/plain"));
}

#[sqlx::test(migrations = "../migrations")]
async fn the_throttle_holds_across_copies_and_a_throttled_wake_comes_back(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    // A 2 s window, so a slow machine still checks inside it.
    let window = Duration::from_secs(2);
    let (here, there) = (
        relay_throttled(&pool, window),
        relay_throttled(&pool, window),
    );
    crate::wake_urls::spawn(here.clone());
    crate::wake_urls::spawn(there.clone());
    let (url, hooked) = server().await;
    put_wake(&here, &w.bob.key, w.b, json!({ "url": url })).await;
    there.heartbeats.reload(&pool).await.unwrap();
    assert!(there.heartbeats.has_wake(w.b));

    invoke(&here, &w, g).await;
    hooked.wait_for(1, 3).await;
    // More work, woken on the other copy, inside the window: no second
    // ping yet, but one once the window ends, since the work still waits.
    invoke(&here, &w, g).await;
    there.wake.notify(w.b, Feed::Invocations);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hooked.count(), 1, "throttled");
    hooked.wait_for(2, 5).await;
    let ping: Value = serde_json::from_str(&hooked.last().1).unwrap();
    assert_eq!(ping["pending"]["invocations"], 2);
    // No reminders: the other copy's throttled wake was covered by that ping.
    tokio::time::sleep(window + Duration::from_millis(800)).await;
    assert_eq!(hooked.count(), 2, "one ping per window, from one copy");
}

#[sqlx::test(migrations = "../migrations")]
async fn twenty_failures_switch_the_url_off(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool);
    let (url, hooked) = server().await;
    hooked.status.store(500, Ordering::Relaxed);
    let path = format!("/v1/agents/{}/wake", w.b);
    put_wake(&state, &w.bob.key, w.b, json!({ "url": url })).await;

    let (status, tested) = relay_post(&state, &w.bob.key, &format!("{path}/test"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{tested}");
    assert_eq!(
        tested,
        json!({ "ok": false, "status": 500, "error": "answered HTTP 500" })
    );
    let (_, read) = get(&state, &w.bob.key, &path).await;
    assert_eq!(
        (read["failures"].clone(), read["enabled"].clone()),
        (json!(1), json!(true))
    );

    sqlx::query("UPDATE agent_wake SET failures = 19 WHERE agent_id = $1")
        .bind(w.b)
        .execute(&pool)
        .await
        .unwrap();
    relay_post(&state, &w.bob.key, &format!("{path}/test"), json!({})).await;
    let (_, read) = get(&state, &w.bob.key, &path).await;
    assert_eq!(read["enabled"], false, "{read}");
    assert!(read["disabled_at"].is_string());
    assert!(!state.heartbeats.has_wake(w.b));
    let (_, notices) = get(
        &state,
        &w.bob.key,
        &format!("/v1/inbox/notices?agent_id={}", w.b),
    )
    .await;
    assert_eq!(notices[0]["kind"], "wake_url_disabled", "{notices}");

    // Setting it again turns it back on.
    hooked.status.store(204, Ordering::Relaxed);
    put_wake(&state, &w.bob.key, w.b, json!({ "url": url })).await;
    let (_, tested) = relay_post(&state, &w.bob.key, &format!("{path}/test"), json!({})).await;
    assert_eq!(tested["ok"], true, "{tested}");
    let (_, read) = get(&state, &w.bob.key, &path).await;
    assert_eq!(
        (read["failures"].clone(), read["enabled"].clone()),
        (json!(0), json!(true))
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_update_agent_sets_and_clears_the_wake_url(pool: PgPool) {
    let w = world(&pool).await;
    let state = relay(&pool);
    let (url, _) = server().await;
    let call = |args: Value| {
        mcp(
            &state,
            &w.bob.key,
            "tools/call",
            json!({ "name": "update_agent", "arguments": args }),
        )
    };
    let set = call(json!({ "agent_id": w.b, "wake_url": url, "wake_format": "text" })).await;
    assert_eq!(set["isError"], false, "{set}");
    let wake = &set["structuredContent"]["wake"];
    assert_eq!(wake["format"], "text");
    assert!(wake["secret"].as_str().unwrap().starts_with("cmw_"));
    assert!(state.heartbeats.has_wake(w.b));

    let cleared = call(json!({ "agent_id": w.b, "wake_url": null })).await;
    assert_eq!(cleared["isError"], false, "{cleared}");
    assert!(cleared["structuredContent"]["wake"].is_null());
    assert!(!state.heartbeats.has_wake(w.b));
}
