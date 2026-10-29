//! `peer` on invocations end to end (waking pull agents §8, W3): a caller
//! sees whether the target is around, as the directory would say, while the
//! invocation is open.

use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::calls::tests::{agent, calls_on, get, grant, mcp, post, world, World};
use crate::handlers::invoke::legacy_v01_contract_tests::config;
use crate::state::RelayState;

/// `agent` last pulled its inbox `ago` (a Postgres interval), as a flush
/// would have written it.
async fn seen(pool: &PgPool, agent: Uuid, ago: &str) {
    sqlx::query(
        "INSERT INTO agent_heartbeats (agent_id, last_polled_at) VALUES ($1, now() - $2::interval)
         ON CONFLICT (agent_id) DO UPDATE SET last_polled_at = EXCLUDED.last_polled_at",
    )
    .bind(agent)
    .bind(ago)
    .execute(pool)
    .await
    .unwrap();
}

/// Alice's agent calls Bob's through `grant_id`.
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

/// What `token` reads for invocation `id`: the row and the list's entry.
async fn read(state: &RelayState, token: &str, id: &str) -> (Value, Value) {
    let (status, one) = get(state, token, &format!("/v1/invocations/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{one}");
    let (status, list) = get(state, token, "/v1/invocations").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let listed = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .cloned()
        .unwrap();
    (one, listed)
}

#[sqlx::test(migrations = "../migrations")]
async fn peer_says_what_the_directory_says(pool: PgPool) {
    let w = world(&pool).await;
    let fresh = agent(&pool, w.bob.account, "fresh").await;
    let lapsed = agent(&pool, w.bob.account, "lapsed").await;
    let never = agent(&pool, w.bob.account, "never").await;
    let pushy = agent(&pool, w.bob.account, "pushy").await;
    sqlx::query("UPDATE agents SET mode = 'push', agent_card_url = 'https://push.example/card' WHERE id = $1")
        .bind(pushy)
        .execute(&pool)
        .await
        .unwrap();
    seen(&pool, fresh, "4 minutes").await;
    seen(&pool, lapsed, "6 minutes").await;

    let mut cfg = config();
    cfg.discovery_v2_enabled = true;
    let state = RelayState::new(pool.clone(), cfg);
    state.heartbeats.reload(&pool).await.unwrap();
    let (status, directory) = get(
        &state,
        &w.alice.key,
        "/v1/discovery/agents?include_dormant=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{directory}");
    let online = |slug: &str| {
        directory["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["agent_slug"] == slug)
            .unwrap_or_else(|| panic!("{slug} not listed: {directory}"))["online"]
            .clone()
    };
    for (slug, id, mode, expect) in [
        ("fresh", fresh, "pull", true),
        ("lapsed", lapsed, "pull", false),
        ("never", never, "pull", false),
    ] {
        let peer = json!(state.heartbeats.peer(id, mode));
        assert_eq!(peer["online"], online(slug), "{slug}");
        assert_eq!(peer["online"], expect, "{slug}");
        assert_eq!(peer["last_seen_at"].is_null(), slug == "never", "{slug}");
    }
    // Push agents don't poll: offline in the directory, and nothing to
    // wait for in `peer`.
    assert_eq!(online("pushy"), false);
    assert_eq!(
        json!(state.heartbeats.peer(pushy, "push")),
        json!({ "delivery": "push" })
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn the_caller_sees_peer_while_the_invocation_is_open(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    seen(&pool, w.b, "3 hours").await;
    let state = calls_on(&pool);
    state.heartbeats.reload(&pool).await.unwrap();

    let queued = invoke(&state, &w, g).await;
    assert_eq!(queued["peer"]["delivery"], "pull", "{queued}");
    assert_eq!(queued["peer"]["online"], false);
    assert!(queued["peer"]["last_seen_at"].is_string());
    let id = queued["invocation_id"].as_str().unwrap().to_owned();

    let (one, listed) = read(&state, &w.alice.key, &id).await;
    assert_eq!(one["peer"], queued["peer"]);
    assert_eq!(listed["peer"], queued["peer"]);
    // The target's side isn't shown it.
    let (one, listed) = read(&state, &w.bob.key, &id).await;
    assert!(one.get("peer").is_none(), "{one}");
    assert!(listed.get("peer").is_none(), "{listed}");

    // Bob's agent pulls: it's online now.
    let (status, pulled) = get(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pulled.as_array().unwrap().len(), 1);
    assert!(pulled[0].get("peer").is_none(), "not on inbox pulls");
    let (one, _) = read(&state, &w.alice.key, &id).await;
    assert_eq!(one["status"], "in_progress");
    assert_eq!(one["peer"]["online"], true, "{one}");

    // Settled: gone.
    let (status, body) = post(
        &state,
        &w.bob.key,
        &format!("/v1/invocations/{id}/result"),
        json!({ "status": "succeeded", "output": { "booked": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (one, listed) = read(&state, &w.alice.key, &id).await;
    assert_eq!(one["status"], "succeeded");
    assert!(one.get("peer").is_none(), "{one}");
    assert!(listed.get("peer").is_none(), "{listed}");
}

#[sqlx::test(migrations = "../migrations")]
async fn a_pull_on_another_copy_shows_after_a_reload(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let (here, there) = (calls_on(&pool), calls_on(&pool));
    let id = invoke(&here, &w, g).await["invocation_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (one, _) = read(&here, &w.alice.key, &id).await;
    assert_eq!(
        one["peer"],
        json!({ "delivery": "pull", "online": false, "last_seen_at": null })
    );

    // Bob's agent pulls from the other copy, which flushes.
    let (status, _) = get(&there, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    assert_eq!(status, StatusCode::OK);
    there.heartbeats.flush(&pool).await.unwrap();
    let (one, _) = read(&here, &w.alice.key, &id).await;
    assert_eq!(one["peer"]["online"], false, "not reloaded yet");

    here.heartbeats.reload(&pool).await.unwrap();
    let (one, _) = read(&here, &w.alice.key, &id).await;
    assert_eq!(one["peer"]["online"], true, "{one}");
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_shows_peer_and_says_when_the_target_may_not_be_around(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    // Half an hour of slack: the database's clock may differ a little.
    seen(&pool, w.b, "3 hours 30 minutes").await;
    let state = calls_on(&pool);
    state.heartbeats.reload(&pool).await.unwrap();
    let handle: String = sqlx::query_scalar(
        "SELECT acc.slug || '/' || a.slug FROM agents a JOIN accounts acc ON acc.id = a.account_id WHERE a.id = $1",
    )
    .bind(w.b)
    .fetch_one(&pool)
    .await
    .unwrap();
    let hint = format!("{handle} was last seen 3 hours ago, so it may not pick this up soon.");
    let call = |name: &str, arguments: Value| {
        mcp(
            &state,
            &w.alice.key,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    };
    let json_of = |result: &Value| -> Value {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let invoked = call(
        "invoke",
        json!({ "grant_id": g, "grantee_agent_id": w.a, "input": { "venue": "Hall A" } }),
    )
    .await;
    assert_eq!(invoked["isError"], false, "{invoked}");
    let body = json_of(&invoked);
    assert_eq!(body["peer"]["online"], false, "{body}");
    assert_eq!(invoked["structuredContent"]["peer"], body["peer"]);
    assert_eq!(invoked["content"][1]["text"], hint, "{invoked}");
    let id = body["invocation_id"].as_str().unwrap().to_owned();

    let polled = call("poll_invocation", json!({ "invocation_id": id })).await;
    assert_eq!(json_of(&polled)["peer"]["online"], false);
    assert_eq!(polled["content"][1]["text"], hint, "{polled}");
    let listed = call("list_invocations", json!({ "direction": "outbound" })).await;
    assert_eq!(json_of(&listed)[0]["peer"]["online"], false, "{listed}");

    // Online: `peer` says so, and there's nothing to add.
    state.heartbeats.record(w.b);
    let polled = call("poll_invocation", json!({ "invocation_id": id })).await;
    assert_eq!(json_of(&polled)["peer"]["online"], true);
    assert_eq!(polled["content"].as_array().unwrap().len(), 1, "{polled}");

    // The target's side isn't shown it.
    let theirs = mcp(
        &state,
        &w.bob.key,
        "tools/call",
        json!({ "name": "poll_invocation", "arguments": { "invocation_id": id } }),
    )
    .await;
    assert!(json_of(&theirs).get("peer").is_none(), "{theirs}");
}
