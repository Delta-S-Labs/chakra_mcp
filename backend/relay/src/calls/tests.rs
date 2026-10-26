//! Calls C1 (spec `2026-10-25-calls-design.md`): the lifecycle, who may
//! call and act, messages and screening, the feed, billing and purging.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::{workers, CallsConfig};
use crate::compliance::test_support::{checker, serve, Fake};
use crate::handlers::invoke::legacy_v01_contract_tests::config;
use crate::state::RelayState;

/// A person with an account, an API key and a web session.
struct Person {
    user: Uuid,
    account: Uuid,
    key: String,
    web: String,
}

async fn person(pool: &PgPool, name: &str) -> Person {
    let user = Uuid::now_v7();
    let account = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, 'x')",
    )
    .bind(user)
    .bind(format!("{name}-{user}@t.local"))
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
         VALUES ($1, $2, $3, 'individual', $4)",
    )
    .bind(account)
    .bind(format!("{}-{account}", name.to_lowercase()))
    .bind(name)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::now_v7())
    .bind(account)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    let key = format!("ck_test_{user}");
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix) VALUES ($1, $2, $3, 't', 'ck_test')",
    )
    .bind(Uuid::now_v7())
    .bind(user)
    .bind(hex::encode(Sha256::digest(key.as_bytes())))
    .execute(pool)
    .await
    .unwrap();
    let claims = chakramcp_shared::jwt::UserClaims::new(user, "u@t.local".into(), false, 1);
    let web = chakramcp_shared::jwt::encode_jwt(&claims, &config().jwt_secret).unwrap();
    Person {
        user,
        account,
        key,
        web,
    }
}

async fn agent(pool: &PgPool, account: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agents (id, account_id, slug, display_name, visibility) VALUES ($1, $2, $3, $3, 'network')",
    )
    .bind(id)
    .bind(account)
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn befriend(pool: &PgPool, a: Uuid, b: Uuid) {
    sqlx::query(
        "INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status, proposer_message, response_message)
         VALUES ($1, $2, $3, 'accepted', 'Let us plan the offsite venue together', 'Sure, venue planning only')",
    )
    .bind(Uuid::now_v7())
    .bind(a)
    .bind(b)
    .execute(pool)
    .await
    .unwrap();
}

/// Alice's agent and Bob's agent, friends.
struct World {
    alice: Person,
    bob: Person,
    a: Uuid,
    b: Uuid,
}

async fn world(pool: &PgPool) -> World {
    let alice = person(pool, "Alice").await;
    let bob = person(pool, "Bob").await;
    let a = agent(pool, alice.account, "alice-agent").await;
    let b = agent(pool, bob.account, "bob-agent").await;
    befriend(pool, a, b).await;
    World { alice, bob, a, b }
}

fn calls_on(pool: &PgPool) -> RelayState {
    RelayState::new(pool.clone(), config()).with_calls(CallsConfig {
        enabled: true,
        ..CallsConfig::default()
    })
}

async fn http(
    state: &RelayState,
    method: Method,
    token: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };
    let res = crate::router(state.clone())
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn post(state: &RelayState, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    http(state, Method::POST, token, uri, Some(body)).await
}

async fn get(state: &RelayState, token: &str, uri: &str) -> (StatusCode, Value) {
    http(state, Method::GET, token, uri, None).await
}

/// Alice's agent invites Bob's; returns the call id.
async fn ring(state: &RelayState, w: &World, extra: Value) -> Uuid {
    let mut body = json!({ "from_agent_id": w.a, "to_agent_id": w.b });
    if let Value::Object(m) = extra {
        body.as_object_mut().unwrap().extend(m);
    }
    let (status, call) = post(state, &w.alice.key, "/v1/calls", body).await;
    assert_eq!(status, StatusCode::CREATED, "{call}");
    call["id"].as_str().unwrap().parse().unwrap()
}

/// A ringing call Bob's agent has accepted.
async fn active(state: &RelayState, w: &World) -> Uuid {
    let id = ring(state, w, json!({})).await;
    let (status, call) = post(
        state,
        &w.bob.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{call}");
    id
}

async fn say(
    state: &RelayState,
    token: &str,
    call: Uuid,
    agent: Uuid,
    text: &str,
) -> (StatusCode, Value) {
    post(
        state,
        token,
        &format!("/v1/calls/{call}/messages"),
        json!({ "agent_id": agent, "text": text }),
    )
    .await
}

/// Pull the agent's feed: `(kind, text)` per item.
async fn pull(state: &RelayState, token: &str, agent: Uuid) -> Vec<(String, Option<String>)> {
    let (status, items) = get(state, token, &format!("/v1/inbox/calls?agent_id={agent}")).await;
    assert_eq!(status, StatusCode::OK, "{items}");
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["entry"]["kind"].as_str().unwrap().to_owned(),
                i["entry"]["text"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn kinds(items: &[(String, Option<String>)]) -> Vec<&str> {
    items.iter().map(|(k, _)| k.as_str()).collect()
}

fn code(body: &Value) -> &str {
    body["error"]["code"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../migrations")]
async fn a_call_rings_is_answered_talks_and_ends(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);

    let (status, call) = post(
        &s,
        &w.alice.key,
        "/v1/calls",
        json!({
            "from_agent_id": w.a, "to_agent_id": w.b,
            "memory": "own", "ttl_secs": 3600, "topic": "Offsite venue",
            "message": "Hi! Can we pick the offsite venue this week?",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{call}");
    assert_eq!(call["state"], "ringing");
    assert_eq!(call["topic"], "Offsite venue");
    assert_eq!(call["message_count"], 1);
    assert_eq!(
        call["memory_features"],
        json!({ "tags": false, "recall": false })
    );
    assert_eq!(call["caller"]["agent_id"], json!(w.a));
    let id = call["id"].as_str().unwrap().to_owned();

    // The invite and the opening message reach Bob's agent, once.
    let items = pull(&s, &w.bob.key, w.b).await;
    assert_eq!(kinds(&items), ["invite", "message"]);
    assert!(pull(&s, &w.bob.key, w.b).await.is_empty());
    assert!(pull(&s, &w.alice.key, w.a).await.is_empty());

    let (status, call) = post(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.b, "message": "Yes, Friday works for a decision." }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{call}");
    assert_eq!(call["state"], "active");
    assert_eq!(call["message_count"], 2);
    assert_eq!(
        kinds(&pull(&s, &w.alice.key, w.a).await),
        ["accepted", "message"]
    );

    let id: Uuid = id.parse().unwrap();
    let (status, entry) = say(
        &s,
        &w.alice.key,
        id,
        w.a,
        "Great, I'll shortlist three venues.",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{entry}");
    assert_eq!(entry["seq"], 5);
    assert_eq!(entry["kind"], "message");
    let items = pull(&s, &w.bob.key, w.b).await;
    assert_eq!(
        items[0].1.as_deref(),
        Some("Great, I'll shortlist three venues.")
    );

    // History: the whole transcript in order, and search by any word.
    let (_, history) = get(&s, &w.alice.key, &format!("/v1/calls/{id}/messages")).await;
    let seqs: Vec<i64> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5]);
    let (_, found) = get(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/messages?q=when%20is%20friday"),
    )
    .await;
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    assert_eq!(found[0]["seq"], 4);
    let (_, after) = get(&s, &w.bob.key, &format!("/v1/calls/{id}/messages?after=3")).await;
    assert_eq!(after.as_array().unwrap().len(), 2);

    let (status, call) = post(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/end"),
        json!({ "agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{call}");
    assert_eq!(call["state"], "ended");
    assert_eq!(call["ended_by"], "callee");
    assert!(call["purge_at"].is_string());
    assert_eq!(kinds(&pull(&s, &w.alice.key, w.a).await), ["ended"]);
    assert!(pull(&s, &w.bob.key, w.b).await.is_empty());

    let (status, body) = say(&s, &w.alice.key, id, w.a, "One more thing").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&body), "chk.calls.not_active");
    let (status, body) = post(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/end"),
        json!({ "agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // Lists: newest first, by agent and state.
    let (_, list) = get(&s, &w.alice.web, "/v1/calls").await;
    assert_eq!(list["calls"].as_array().unwrap().len(), 1);
    let (_, list) = get(
        &s,
        &w.alice.key,
        &format!("/v1/calls?agent_id={}&state=active", w.a),
    )
    .await;
    assert!(list["calls"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "../migrations")]
async fn calls_are_declined_missed_idled_and_ended_by_owners(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);

    let id = ring(&s, &w, json!({})).await;
    let (status, call) = post(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/decline"),
        json!({ "agent_id": w.b, "reason": "Busy this quarter" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{call}");
    assert_eq!(call["state"], "declined");
    let items = pull(&s, &w.alice.key, w.a).await;
    assert_eq!(
        items,
        [("declined".to_owned(), Some("Busy this quarter".to_owned()))]
    );
    // The caller can't answer its own call.
    let id = ring(&s, &w, json!({})).await;
    let (status, _) = post(
        &s,
        &w.alice.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.a }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Unanswered for a day: missed.
    sqlx::query("UPDATE calls SET created_at = now() - interval '25 hours' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    workers::tick(&pool).await.unwrap();
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(call["state"], "missed");
    assert_eq!(kinds(&pull(&s, &w.alice.key, w.a).await), ["missed"]);

    // Quiet past its idle timeout: ended, and both are told.
    let id = active(&s, &w).await;
    pull(&s, &w.alice.key, w.a).await;
    pull(&s, &w.bob.key, w.b).await;
    sqlx::query("UPDATE calls SET last_message_at = now() - interval '2 days' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    workers::tick(&pool).await.unwrap();
    let (_, call) = get(&s, &w.bob.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(call["ended_by"], "idle");
    assert_eq!(kinds(&pull(&s, &w.alice.key, w.a).await), ["ended"]);
    assert_eq!(kinds(&pull(&s, &w.bob.key, w.b).await), ["ended"]);

    // An owner ends a call from the web; an API key can't.
    let id = active(&s, &w).await;
    pull(&s, &w.alice.key, w.a).await;
    pull(&s, &w.bob.key, w.b).await;
    let (status, body) = post(
        &s,
        &w.alice.key,
        &format!("/v1/calls/{id}/owner-end"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&body), "chk.calls.owner_only");
    let (status, call) = post(
        &s,
        &w.alice.web,
        &format!("/v1/calls/{id}/owner-end"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{call}");
    assert_eq!(call["ended_by"], "owner");
    assert_eq!(kinds(&pull(&s, &w.alice.key, w.a).await), ["ended"]);
    assert_eq!(kinds(&pull(&s, &w.bob.key, w.b).await), ["ended"]);

    // A party that's deleted ends its calls.
    let id = active(&s, &w).await;
    sqlx::query("UPDATE agents SET tombstoned_at = now() WHERE id = $1")
        .bind(w.b)
        .execute(&pool)
        .await
        .unwrap();
    workers::tick(&pool).await.unwrap();
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(call["ended_by"], "gone");
}

#[sqlx::test(migrations = "../migrations")]
async fn who_may_call_whom(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let carol = person(&pool, "Carol").await;
    let c = agent(&pool, carol.account, "carol-agent").await;
    let invite = |token: String, from: Uuid, to: Uuid| {
        let s = s.clone();
        async move {
            post(
                &s,
                &token,
                "/v1/calls",
                json!({ "from_agent_id": from, "to_agent_id": to }),
            )
            .await
        }
    };

    // Friends only, by default.
    let (status, body) = invite(carol.key.clone(), c, w.b).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&body), "chk.calls.not_accepted");
    let (status, updated) = http(
        &s,
        Method::PATCH,
        &w.bob.key,
        &format!("/v1/agents/{}", w.b),
        Some(json!({ "accept_calls": "anyone" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["accept_calls"], "anyone");
    let (status, _) = invite(carol.key.clone(), c, w.b).await;
    assert_eq!(status, StatusCode::CREATED);

    // Nobody: not even a friend.
    http(
        &s,
        Method::PATCH,
        &w.bob.key,
        &format!("/v1/agents/{}", w.b),
        Some(json!({ "accept_calls": "nobody" })),
    )
    .await;
    let (status, _) = invite(w.alice.key.clone(), w.a, w.b).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = http(
        &s,
        Method::PATCH,
        &w.bob.key,
        &format!("/v1/agents/{}", w.b),
        Some(json!({ "accept_calls": "everyone" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Agents of one account may call each other, friends or not.
    let a2 = agent(&pool, w.alice.account, "alice-second").await;
    let (status, _) = invite(w.alice.key.clone(), a2, w.a).await;
    assert_eq!(status, StatusCode::CREATED);

    // Yourself, a tombstoned agent, a push agent.
    let (status, body) = invite(w.alice.key.clone(), w.a, w.a).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "chk.calls.bad_terms")
    );
    let gone = agent(&pool, carol.account, "carol-gone").await;
    sqlx::query("UPDATE agents SET tombstoned_at = now() WHERE id = $1")
        .bind(gone)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = invite(carol.key.clone(), c, gone).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let push = agent(&pool, carol.account, "carol-push").await;
    sqlx::query("UPDATE agents SET mode = 'push', agent_card_url = 'https://push.example/card.json' WHERE id = $1")
        .bind(push)
        .execute(&pool)
        .await
        .unwrap();
    let (status, body) = invite(carol.key.clone(), c, push).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.push_unsupported")
    );
    let (status, body) = http(
        &s,
        Method::PATCH,
        &carol.key,
        &format!("/v1/agents/{push}"),
        Some(json!({ "accept_calls": "anyone" })),
    )
    .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.push_unsupported")
    );
    let (_, shown) = get(&s, &carol.key, &format!("/v1/agents/{push}")).await;
    assert_eq!(shown["accept_calls"], "nobody");

    // Busy: one ringing call per pair, and the per-agent cap.
    http(
        &s,
        Method::PATCH,
        &w.bob.key,
        &format!("/v1/agents/{}", w.b),
        Some(json!({ "accept_calls": "friends" })),
    )
    .await;
    let (status, _) = invite(w.alice.key.clone(), w.a, w.b).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = invite(w.alice.key.clone(), w.a, w.b).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.busy")
    );
    let small = RelayState::new(pool.clone(), config()).with_calls(CallsConfig {
        enabled: true,
        max_active_per_agent: 1,
        ..CallsConfig::default()
    });
    let d = agent(&pool, w.alice.account, "alice-third").await;
    let (status, body) = post(
        &small,
        &w.alice.key,
        "/v1/calls",
        json!({ "from_agent_id": w.a, "to_agent_id": d }),
    )
    .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.busy")
    );
    // Invites others send don't count against an agent's own calls: Bob's
    // agent has two ringing invites and a cap of 1, and can still call.
    let (status, body) = post(
        &small,
        &w.bob.key,
        "/v1/calls",
        json!({ "from_agent_id": w.b, "to_agent_id": w.a }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[sqlx::test(migrations = "../migrations")]
async fn parties_act_with_agent_credentials_and_owners_from_the_web(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);

    // A web session can't act as the agent.
    let (status, body) = post(
        &s,
        &w.alice.web,
        "/v1/calls",
        json!({ "from_agent_id": w.a, "to_agent_id": w.b }),
    )
    .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "chk.calls.owner_session")
    );

    // A key scoped to another agent can't either.
    let b2 = agent(&pool, w.bob.account, "bob-second").await;
    let key_id: Uuid = sqlx::query_scalar("SELECT id FROM api_keys WHERE user_id = $1")
        .bind(w.bob.user)
        .fetch_one(&pool)
        .await
        .unwrap();
    let scope = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO credential_scopes (id, cred_kind, cred_ref, user_id, agent_scope)
         VALUES ($1, 'api_key', $2, $3, 'selected')",
    )
    .bind(scope)
    .bind(key_id.to_string())
    .bind(w.bob.user)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO credential_scope_agents (credential_scope_id, agent_id) VALUES ($1, $2)",
    )
    .bind(scope)
    .bind(b2)
    .execute(&pool)
    .await
    .unwrap();
    let id = ring(&s, &w, json!({})).await;
    let (status, _) = post(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = get(&s, &w.bob.key, &format!("/v1/inbox/calls?agent_id={}", w.b)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Reading: either party's members, by key or on the web; nobody else.
    let carol = person(&pool, "Carol").await;
    let (status, _) = get(&s, &carol.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&s, &carol.key, &format!("/v1/calls/{id}/messages")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post(
        &s,
        &carol.web,
        &format!("/v1/calls/{id}/owner-end"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for token in [&w.alice.web, &w.alice.key, &w.bob.web] {
        let (status, _) = get(&s, token, &format!("/v1/calls/{id}")).await;
        assert_eq!(status, StatusCode::OK);
    }
}

#[sqlx::test(migrations = "../migrations")]
async fn messages_are_limited_screened_and_charged(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let id = active(&s, &w).await;
    pull(&s, &w.bob.key, w.b).await;

    // At most 16 KiB.
    let (status, body) = say(&s, &w.alice.key, id, w.a, &"x".repeat(17 * 1024)).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
    );

    // 30 a minute per sender.
    for seq in 100..130 {
        sqlx::query(
            "INSERT INTO call_entries (id, call_id, seq, kind, from_agent_id, text) VALUES ($1, $2, $3, 'message', $4, 'hi')",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(seq)
        .bind(w.a)
        .execute(&pool)
        .await
        .unwrap();
    }
    let (status, body) = say(&s, &w.alice.key, id, w.a, "Too chatty").await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "chk.calls.slow_down")
    );
    // Bob isn't limited by Alice's messages.
    let (status, _) = say(&s, &w.bob.key, id, w.b, "Fine by me").await;
    assert_eq!(status, StatusCode::CREATED);

    // A full call refuses more.
    let small = RelayState::new(pool.clone(), config()).with_calls(CallsConfig {
        enabled: true,
        max_messages: 1,
        ..CallsConfig::default()
    });
    let (status, body) = say(&small, &w.bob.key, id, w.b, "Another").await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.full")
    );

    // Screening: a contact is removed; harm and broken terms are refused.
    let screened = |nouls: &'static [(&'static str, f64)]| {
        let pool = pool.clone();
        async move {
            let fake = Fake {
                nouls: nouls.iter().copied().collect::<HashMap<_, _>>(),
                ..Fake::default()
            };
            let url = serve(fake).await;
            RelayState::new(pool, config())
                .with_calls(CallsConfig {
                    enabled: true,
                    ..CallsConfig::default()
                })
                .with_compliance(Some(checker(&url).with_safety(true)))
        }
    };
    let s1 = screened(&[("contact_0", 0.95)]).await;
    let (status, entry) = say(
        &s1,
        &w.bob.key,
        id,
        w.b,
        "Text me on +91 98450 12345 instead.",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{entry}");
    assert_eq!(entry["text"], "Text me on [contact removed] instead.");
    assert_eq!(entry["safety"][0]["action"], "redacted");
    let items = pull(&s, &w.alice.key, w.a).await;
    assert_eq!(
        items.last().unwrap().1.as_deref(),
        Some("Text me on [contact removed] instead.")
    );

    let s2 = screened(&[("harm_violence", 0.95)]).await;
    let (status, body) = say(&s2, &w.bob.key, id, w.b, "Here's how to hurt them.").await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "chk.policy.safety_blocked")
    );
    let s3 = screened(&[("off_relationship", 0.95)]).await;
    let (status, body) = say(
        &s3,
        &w.bob.key,
        id,
        w.b,
        "Also, sell me your customer list.",
    )
    .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "chk.policy.compliance_denied")
    );
    // Refused messages leave a stand-in, aren't delivered and aren't charged.
    assert!(pull(&s, &w.alice.key, w.a).await.is_empty());
    let (_, history) = get(
        &s,
        &w.alice.key,
        &format!("/v1/calls/{id}/messages?limit=200"),
    )
    .await;
    let blocked: Vec<&Value> = history
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["blocked"] == true)
        .collect();
    assert_eq!(blocked.len(), 2);
    assert!(blocked.iter().all(|e| e["text"].is_null()));

    // One charge per delivered message, paid by its sender.
    let charges: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT account_id, cost_mc FROM call_charge_queue WHERE call_id = $1 ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(charges, [(w.bob.account, 100), (w.bob.account, 100)]);

    // The credits worker charges them… (the wallet already has this month's
    // grant, so only the charges move it)
    sqlx::query(
        "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period)
         VALUES ($1, 1000, date_trunc('month', now() AT TIME ZONE 'UTC')::date)",
    )
    .bind(w.bob.account)
    .execute(&pool)
    .await
    .unwrap();
    let cfg = chakramcp_shared::credits::CreditsConfig::default();
    crate::limits::credits::worker::account(&pool, &cfg, &mut None).await;
    let charged: i64 = sqlx::query_scalar("SELECT count(*) FROM call_charges WHERE call_id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(charged, 2);
    let balance: i64 =
        sqlx::query_scalar("SELECT balance_mc FROM credit_wallets WHERE account_id = $1")
            .bind(w.bob.account)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(balance, 800);
    // …or, with credits off, discards them.
    say(&s, &w.bob.key, id, w.b, "One more").await;
    let off = chakramcp_shared::credits::CreditsConfig {
        enabled: false,
        ..cfg
    };
    crate::limits::credits::worker::account(&pool, &off, &mut None).await;
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM call_charge_queue")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(queued, 0);
}

#[sqlx::test(migrations = "../migrations")]
async fn content_is_deleted_at_the_ttl_or_by_an_owner(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let id = ring(
        &s,
        &w,
        json!({ "ttl_secs": 0, "topic": "Venue", "message": "Hello" }),
    )
    .await;
    post(
        &s,
        &w.bob.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.b }),
    )
    .await;
    say(&s, &w.bob.key, id, w.b, "Budget is 4000").await;
    // An invocation made in the call, still waiting for its agent.
    let inv = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO relay_invocations (id, capability_name, status, input_preview, call_id)
         VALUES ($1, 'book_venue', 'pending', '{\"venue\": \"Hall A\"}', $2)",
    )
    .bind(inv)
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    post(
        &s,
        &w.alice.key,
        &format!("/v1/calls/{id}/end"),
        json!({ "agent_id": w.a }),
    )
    .await;

    let purged = |pool: PgPool| async move {
        sqlx::query_scalar::<_, bool>("SELECT purged_at IS NOT NULL FROM calls WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    // TTL 0, but Alice hasn't pulled Bob's message yet: it waits.
    workers::tick(&pool).await.unwrap();
    assert!(!purged(pool.clone()).await);
    pull(&s, &w.alice.key, w.a).await;
    pull(&s, &w.bob.key, w.b).await;
    // Delivered, but the invocation is still pending: it waits.
    workers::tick(&pool).await.unwrap();
    assert!(!purged(pool.clone()).await);
    // A week past the TTL, the invocation is closed and the call purged.
    sqlx::query("UPDATE calls SET purge_at = now() - interval '8 days' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    workers::tick(&pool).await.unwrap();
    assert!(purged(pool.clone()).await);
    let (status, error, input): (String, Option<String>, Option<Value>) = sqlx::query_as(
        "SELECT status, error_message, input_preview FROM relay_invocations WHERE id = $1",
    )
    .bind(inv)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(error.as_deref(), Some(super::purge::CLOSED));
    assert!(input.is_none());
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert!(call["topic"].is_null());
    assert!(call["purged_at"].is_string());
    let (_, history) = get(&s, &w.alice.key, &format!("/v1/calls/{id}/messages")).await;
    let entries = history.as_array().unwrap();
    assert!(entries.iter().all(|e| e["text"].is_null()), "{history}");
    assert_eq!(entries.len(), 5, "the stub keeps every entry");

    // An owner deletes an active call's content at once; a held request is
    // rejected without waiting for its approver.
    let id = active(&s, &w).await;
    pull(&s, &w.bob.key, w.b).await;
    say(&s, &w.alice.key, id, w.a, "Secret plan").await;
    let held = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO relay_invocations (id, capability_name, status, input_preview, call_id)
         VALUES ($1, 'book_venue', 'awaiting_consent', '{}', $2)",
    )
    .bind(held)
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let (status, _) = http(
        &s,
        Method::DELETE,
        &w.bob.key,
        &format!("/v1/calls/{id}/content"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = http(
        &s,
        Method::DELETE,
        &w.bob.web,
        &format!("/v1/calls/{id}/content"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(call["ended_by"], "owner");
    assert!(call["purged_at"].is_string());
    let held_status: String =
        sqlx::query_scalar("SELECT status FROM relay_invocations WHERE id = $1")
            .bind(held)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(held_status, "rejected");
    // The message is gone; the end is still delivered.
    assert_eq!(kinds(&pull(&s, &w.bob.key, w.b).await), ["ended"]);
}

#[sqlx::test(migrations = "../migrations")]
async fn calls_follow_agents_that_move_or_go(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let id = active(&s, &w).await;

    // Bob's agent moves to Dave's account: Dave sees the call and pays for
    // the agent's messages; Bob no longer sees it.
    let dave = person(&pool, "Dave").await;
    sqlx::query("UPDATE agents SET account_id = $2 WHERE id = $1")
        .bind(w.b)
        .bind(dave.account)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = get(&s, &dave.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&s, &w.bob.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = say(&s, &dave.key, id, w.b, "Dave's agent now").await;
    assert_eq!(status, StatusCode::CREATED);
    let payer: Uuid =
        sqlx::query_scalar("SELECT account_id FROM call_charge_queue WHERE call_id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payer, dave.account);

    // A deleted agent (an org's deletion cascades to its tombstoned agents)
    // leaves the call in place; the worker ends it.
    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(w.b)
        .execute(&pool)
        .await
        .unwrap();
    let (status, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(call["callee"]["agent_id"].is_null());
    workers::tick(&pool).await.unwrap();
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(call["ended_by"], "gone");
}

#[sqlx::test(migrations = "../migrations")]
async fn calls_are_404_while_switched_off(pool: PgPool) {
    let w = world(&pool).await;
    let off = RelayState::new(pool.clone(), config());
    let (status, _) = post(
        &off,
        &w.alice.key,
        "/v1/calls",
        json!({ "from_agent_id": w.a, "to_agent_id": w.b }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(
        &off,
        &w.alice.key,
        &format!("/v1/inbox/calls?agent_id={}", w.a),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Agents show the setting either way.
    let (_, shown) = get(&off, &w.alice.key, &format!("/v1/agents/{}", w.a)).await;
    assert_eq!(shown["accept_calls"], "friends");
}

#[sqlx::test(migrations = "../migrations")]
async fn concurrent_messages_get_distinct_ordered_seqs(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let id = active(&s, &w).await;
    let mut tasks = Vec::new();
    for i in 0..12 {
        let (s, token, agent) = if i % 2 == 0 {
            (s.clone(), w.alice.key.clone(), w.a)
        } else {
            (s.clone(), w.bob.key.clone(), w.b)
        };
        tasks.push(tokio::spawn(async move {
            say(&s, &token, id, agent, &format!("message {i}")).await.0
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), StatusCode::CREATED);
    }
    let (_, history) = get(&s, &w.alice.key, &format!("/v1/calls/{id}/messages")).await;
    let seqs: Vec<i64> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, (1..=14).collect::<Vec<i64>>());
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(call["message_count"], 12);
}

// ─── C2: capabilities in a call, and the MCP tools ───────

/// A capability on `granter`, granted to `grantee`; the grant's id.
async fn grant(pool: &PgPool, granter: Uuid, grantee: Uuid, name: &str) -> Uuid {
    let cap = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agent_capabilities (id, agent_id, name, description, input_schema, output_schema, visibility)
         VALUES ($1, $2, $3, 'Book a venue for an event.', '{\"type\":\"object\"}', '{\"type\":\"object\"}', 'network')",
    )
    .bind(cap)
    .bind(granter)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    let grant = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO grants (id, granter_agent_id, grantee_agent_id, capability_id, status)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(grant)
    .bind(granter)
    .bind(grantee)
    .bind(cap)
    .execute(pool)
    .await
    .unwrap();
    grant
}

async fn invoke_in(
    state: &RelayState,
    token: &str,
    grant_id: Uuid,
    grantee: Uuid,
    call: Uuid,
) -> (StatusCode, Value) {
    post(
        state,
        token,
        "/v1/invoke",
        json!({
            "grant_id": grant_id, "grantee_agent_id": grantee,
            "input": { "venue": "Hall A", "date": "2026-11-14" },
            "call_id": call,
        }),
    )
    .await
}

#[sqlx::test(migrations = "../migrations")]
async fn capabilities_are_requested_inside_a_call(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let book = grant(&pool, w.b, w.a, "book_venue").await;
    let id = active(&s, &w).await;

    let (status, body) = invoke_in(&s, &w.alice.key, book, w.a, id).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let inv = body["invocation_id"].as_str().unwrap().to_owned();
    let (_, row) = get(&s, &w.alice.key, &format!("/v1/invocations/{inv}")).await;
    assert_eq!(row["call_id"], json!(id));
    // Bob's agent gets it through its inbox, with the call it's part of.
    let (_, inbox) = get(&s, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    assert_eq!(inbox[0]["call_id"], json!(id), "{inbox}");
    // The transcript shows it, with its status as it is now.
    let (_, history) = get(&s, &w.alice.key, &format!("/v1/calls/{id}/messages")).await;
    let entry = history.as_array().unwrap().last().unwrap();
    assert_eq!(entry["kind"], "invocation");
    assert_eq!(entry["from_agent_id"], json!(w.a));
    assert_eq!(entry["invocation"]["capability"], "book_venue");
    assert_eq!(entry["invocation"]["status"], "in_progress");
    // Not delivered on the calls feed: it went through the inbox.
    assert!(pull(&s, &w.bob.key, w.b)
        .await
        .iter()
        .all(|(k, _)| k != "invocation"));

    // Only between the call's two parties, on an active call, by the agent.
    let a2 = agent(&pool, w.alice.account, "alice-second").await;
    befriend(&pool, w.b, a2).await;
    let other_grantee = grant(&pool, w.b, a2, "book_room").await;
    let (status, body) = invoke_in(&s, &w.alice.key, other_grantee, a2, id).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "chk.calls.not_a_party")
    );
    let carol = person(&pool, "Carol").await;
    let c = agent(&pool, carol.account, "carol-agent").await;
    befriend(&pool, c, w.a).await;
    let third = grant(&pool, c, w.a, "cater").await;
    let (status, body) = invoke_in(&s, &w.alice.key, third, w.a, id).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "chk.calls.not_a_party")
    );
    let (status, body) = invoke_in(&s, &w.alice.web, book, w.a, id).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "chk.calls.owner_session")
    );
    let (status, _) = invoke_in(&s, &carol.key, book, w.a, id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let off = RelayState::new(pool.clone(), config());
    let (status, body) = invoke_in(&off, &w.alice.key, book, w.a, id).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "chk.calls.bad_terms")
    );
    post(
        &s,
        &w.alice.key,
        &format!("/v1/calls/{id}/end"),
        json!({ "agent_id": w.a }),
    )
    .await;
    let (status, body) = invoke_in(&s, &w.alice.key, book, w.a, id).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.not_active")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn a_refused_request_still_joins_its_call(pool: PgPool) {
    let w = world(&pool).await;
    let book = grant(&pool, w.b, w.a, "book_venue").await;
    let id = active(&calls_on(&pool), &w).await;
    let fake = Fake {
        nouls: [("off_capability", 0.95)]
            .into_iter()
            .collect::<HashMap<_, _>>(),
        ..Fake::default()
    };
    let url = serve(fake).await;
    let s = calls_on(&pool).with_compliance(Some(checker(&url)));
    let (status, body) = invoke_in(&s, &w.alice.key, book, w.a, id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (_, history) = get(&s, &w.alice.key, &format!("/v1/calls/{id}/messages")).await;
    let entry = history.as_array().unwrap().last().unwrap();
    assert_eq!(entry["kind"], "invocation");
    assert_eq!(entry["invocation"]["status"], "rejected");
}

async fn mcp(state: &RelayState, token: &str, method: &str, params: Value) -> Value {
    let (status, body) = post(
        state,
        token,
        "/mcp",
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"].clone()
}

/// A tool's result as JSON (`structuredContent`, or the text for arrays).
async fn tool(state: &RelayState, token: &str, name: &str, arguments: Value) -> Value {
    let result = mcp(
        state,
        token,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await;
    assert_eq!(result["isError"], false, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[sqlx::test(migrations = "../migrations")]
async fn agents_hold_calls_over_mcp(pool: PgPool) {
    let w = world(&pool).await;
    let s = calls_on(&pool);
    let listed = mcp(&s, &w.alice.key, "tools/list", json!({})).await;
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in [
        "start_call",
        "answer_call",
        "send_message",
        "pull_calls",
        "call_history",
        "end_call",
    ] {
        assert!(names.contains(&name), "{name} missing");
    }

    let call = tool(&s, &w.alice.key, "start_call", json!({
        "from_agent_id": w.a, "to_agent_id": w.b, "topic": "Venue", "message": "Shall we pick a venue?",
    }))
    .await;
    let id = call["id"].as_str().unwrap().to_owned();
    let items = tool(&s, &w.bob.key, "pull_calls", json!({ "agent_id": w.b })).await;
    assert_eq!(items.as_array().unwrap().len(), 2);
    let call = tool(
        &s,
        &w.bob.key,
        "answer_call",
        json!({
            "call_id": id, "agent_id": w.b, "answer": "accept", "message": "Yes, Hall A?",
        }),
    )
    .await;
    assert_eq!(call["state"], "active");
    let entry = tool(
        &s,
        &w.alice.key,
        "send_message",
        json!({
            "call_id": id, "agent_id": w.a, "text": "Hall A it is.",
        }),
    )
    .await;
    assert_eq!(entry["kind"], "message");
    let found = tool(
        &s,
        &w.bob.key,
        "call_history",
        json!({ "call_id": id, "q": "hall" }),
    )
    .await;
    assert_eq!(found.as_array().unwrap().len(), 2, "{found}");
    let got = tool(&s, &w.alice.key, "get_call", json!({ "call_id": id })).await;
    assert_eq!(got["message_count"], 3);
    let call = tool(
        &s,
        &w.bob.key,
        "end_call",
        json!({ "call_id": id, "agent_id": w.b }),
    )
    .await;
    assert_eq!(call["state"], "ended");
    let list = tool(&s, &w.alice.key, "list_calls", json!({})).await;
    assert_eq!(list["calls"].as_array().unwrap().len(), 1);

    // Refusals come back as tool errors with the code.
    let result = mcp(&s, &w.alice.key, "tools/call", json!({
        "name": "send_message", "arguments": { "call_id": id, "agent_id": w.a, "text": "Hello?" },
    }))
    .await;
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "chk.calls.not_active");

    // With calls off, the tools aren't there.
    let off = RelayState::new(pool.clone(), config());
    let listed = mcp(&off, &w.alice.key, "tools/list", json!({})).await;
    assert!(!listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "start_call"));
    let (_, body) = post(&off, &w.alice.key, "/mcp", json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "start_call", "arguments": { "from_agent_id": w.a, "to_agent_id": w.b } },
    }))
    .await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown tool"),
        "{body}"
    );
}

// ─── C3: call memory ─────────────────────────────────────

/// Jev's answers for the memory tests: seq 2 asks which venue, seq 4
/// decides it ("Hall A works, agreed.") and answers seq 2; recall picks 4.
fn memory_fake() -> Fake {
    Fake {
        nouls: [("answered", 0.93)].into_iter().collect::<HashMap<_, _>>(),
        answer: Some(std::sync::Arc::new(|id: &str, _q: &Value| match id {
            "m2_open_question" => Some(json!({ "type": "noul", "noul": 0.9 })),
            "m4_decision" => Some(json!({ "type": "noul", "noul": 0.95 })),
            "m4_weight" => Some(json!({ "type": "score", "score": 3.0 })),
            "m4_resolves" => Some(json!({
                "type": "choice", "choice": "m2", "probabilities": { "m2": 0.9, "none": 0.1 }
            })),
            "best" => Some(json!({
                "type": "choice", "choice": "m4",
                "probabilities": { "m4": 0.8, "m2": 0.1, "none": 0.1 }
            })),
            _ => None,
        })),
        ..Fake::default()
    }
}

async fn memory_on(pool: &PgPool, fake: Fake) -> RelayState {
    let url = serve(fake).await;
    calls_on(pool).with_memory(Some(checker(&url)))
}

/// A memory call: Alice asks which venue (seq 2), Bob accepts (3) and
/// decides (4), then eight long messages push the start out of `recent`.
async fn memory_call(s: &RelayState, w: &World) -> Uuid {
    let id = ring(
        s,
        w,
        json!({ "memory": "call", "topic": "Offsite", "message": "Which venue should we book?" }),
    )
    .await;
    post(
        s,
        &w.bob.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.b, "message": "Hall A works, agreed." }),
    )
    .await;
    let filler = "We talked through the agenda in detail. ".repeat(40);
    for i in 0..8 {
        let (token, agent) = if i % 2 == 0 {
            (&w.alice.key, w.a)
        } else {
            (&w.bob.key, w.b)
        };
        let (status, _) = say(s, token, id, agent, &filler).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    id
}

#[sqlx::test(migrations = "../migrations")]
async fn memory_tags_messages_and_builds_the_bundle(pool: PgPool) {
    let w = world(&pool).await;
    let s = memory_on(&pool, memory_fake()).await;
    let id = memory_call(&s, &w).await;
    let (_, call) = get(&s, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(
        call["memory_features"],
        json!({ "tags": true, "recall": true })
    );
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM call_entries WHERE call_id = $1 AND tag_state = 'pending'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 10);

    let tagged = super::memory::tagger::tag_once(&s).await.unwrap();
    assert_eq!(tagged, 10);
    let tags: Value =
        sqlx::query_scalar("SELECT tags FROM call_entries WHERE call_id = $1 AND seq = 4")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tags["decision"], 0.95);
    assert_eq!(tags["resolves"], 2);
    // Metered and charged to the caller.
    let (kind, cost): (String, i64) =
        sqlx::query_as("SELECT kind, cost_mc FROM call_memory_requests WHERE call_id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), cost), ("tag", 42));
    let payer: Uuid = sqlx::query_scalar(
        "SELECT account_id FROM call_charge_queue WHERE call_id = $1 AND kind = 'memory'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payer, w.alice.account);
    // Nothing left to tag, nothing tagged twice.
    assert_eq!(super::memory::tagger::tag_once(&s).await.unwrap(), 0);

    // The bundle: the decision pinned with what it answers; the question
    // answered, so not open; the latest messages recent.
    let (status, memory) = get(&s, &w.bob.key, &format!("/v1/calls/{id}/memory")).await;
    assert_eq!(status, StatusCode::OK, "{memory}");
    assert_eq!(memory["pinned"][0]["seq"], 4);
    assert_eq!(memory["pinned"][0]["text"], "Hall A works, agreed.");
    assert_eq!(
        memory["pinned"][0]["context"],
        "Which venue should we book?"
    );
    assert!(memory["open"].as_array().unwrap().is_empty());
    let recent = memory["recent"].as_array().unwrap();
    assert_eq!(recent.last().unwrap()["seq"], 12);

    // The feed carries one bundle per call, on the newest message.
    let (_, items) = get(
        &s,
        &w.alice.key,
        &format!("/v1/inbox/calls?agent_id={}", w.a),
    )
    .await;
    let with_memory: Vec<&Value> = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i.get("memory").is_some())
        .collect();
    assert_eq!(with_memory.len(), 1);
    assert_eq!(with_memory[0]["entry"]["seq"], 12);
    assert_eq!(with_memory[0]["memory"]["pinned"][0]["seq"], 4);
}

#[sqlx::test(migrations = "../migrations")]
async fn recall_finds_the_message_that_answers(pool: PgPool) {
    let w = world(&pool).await;
    let s = memory_on(&pool, memory_fake()).await;
    let id = memory_call(&s, &w).await;
    super::memory::tagger::tag_once(&s).await.unwrap();
    let ask = |token: String, agent: Uuid| {
        let s = s.clone();
        async move {
            post(
                &s,
                &token,
                &format!("/v1/calls/{id}/memory/ask"),
                json!({
                    "agent_id": agent, "question": "Which venue did we agree on?",
                }),
            )
            .await
        }
    };
    let (status, answer) = ask(w.bob.key.clone(), w.b).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["answered"], 0.93);
    assert_eq!(answer["messages"][0]["seq"], 4);
    assert_eq!(answer["messages"][0]["text"], "Hall A works, agreed.");
    assert_eq!(answer["messages"][0]["from"], json!(w.b));
    // Logged against Bob's agent, paid for by the caller.
    let asker: Uuid = sqlx::query_scalar(
        "SELECT agent_id FROM call_memory_requests WHERE call_id = $1 AND kind = 'recall'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(asker, w.b);
    let payers: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT account_id FROM call_charge_queue WHERE call_id = $1 AND kind = 'memory'",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(payers, [w.alice.account]);

    // 30 an hour per agent.
    for _ in 0..30 {
        sqlx::query("INSERT INTO call_memory_requests (id, call_id, agent_id, kind) VALUES ($1, $2, $3, 'recall')")
            .bind(Uuid::now_v7())
            .bind(id)
            .bind(w.b)
            .execute(&pool)
            .await
            .unwrap();
    }
    let (status, body) = ask(w.bob.key.clone(), w.b).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::TOO_MANY_REQUESTS, "chk.calls.slow_down")
    );
    let (status, _) = ask(w.alice.key.clone(), w.a).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Alice's agent has its own allowance"
    );

    // Over MCP too.
    let found = tool(
        &s,
        &w.alice.key,
        "ask_call_memory",
        json!({
            "call_id": id, "agent_id": w.a, "question": "Which venue?",
        }),
    )
    .await;
    assert_eq!(found["messages"][0]["seq"], 4);
    let bundle = tool(&s, &w.alice.key, "call_memory", json!({ "call_id": id })).await;
    assert_eq!(bundle["pinned"][0]["seq"], 4);

    // Deleted content can't be asked.
    http(
        &s,
        Method::DELETE,
        &w.alice.web,
        &format!("/v1/calls/{id}/content"),
        None,
    )
    .await;
    let (status, body) = ask(w.alice.key.clone(), w.a).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "chk.calls.not_active")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn calls_without_memory_have_none(pool: PgPool) {
    let w = world(&pool).await;
    let on = memory_on(&pool, memory_fake()).await;
    // memory: own.
    let id = ring(&on, &w, json!({ "message": "Hi" })).await;
    let (status, body) = get(&on, &w.alice.key, &format!("/v1/calls/{id}/memory")).await;
    assert_eq!(
        (status, code(&body)),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "chk.calls.memory_unavailable"
        )
    );
    post(
        &on,
        &w.alice.key,
        &format!("/v1/calls/{id}/end"),
        json!({ "agent_id": w.a }),
    )
    .await;
    // memory: call, placed with memory off: recent only, nothing pending,
    // and no recall.
    let off = calls_on(&pool);
    let id = ring(&off, &w, json!({ "memory": "call", "message": "Hi there" })).await;
    let (_, call) = get(&off, &w.alice.key, &format!("/v1/calls/{id}")).await;
    assert_eq!(
        call["memory_features"],
        json!({ "tags": false, "recall": false })
    );
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM call_entries WHERE call_id = $1 AND tag_state IS NOT NULL",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 0);
    let (status, memory) = get(&off, &w.alice.key, &format!("/v1/calls/{id}/memory")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(memory["recent"][0]["text"], "Hi there");
    post(
        &off,
        &w.bob.key,
        &format!("/v1/calls/{id}/accept"),
        json!({ "agent_id": w.b }),
    )
    .await;
    let (status, body) = post(
        &on,
        &w.bob.key,
        &format!("/v1/calls/{id}/memory/ask"),
        json!({ "agent_id": w.b, "question": "Hi?" }),
    )
    .await;
    assert_eq!(
        (status, code(&body)),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "chk.calls.memory_unavailable"
        )
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn tagging_retries_gives_up_and_waits_for_credits(pool: PgPool) {
    let w = world(&pool).await;
    let failing = Fake {
        status: Some(500),
        ..Fake::default()
    };
    let s = memory_on(&pool, failing).await;
    let id = ring(
        &s,
        &w,
        json!({ "memory": "call", "message": "Which venue?" }),
    )
    .await;
    // Fails: the attempt is counted and the lease holds it for 5 minutes.
    assert_eq!(super::memory::tagger::tag_once(&s).await.unwrap(), 0);
    let (state, tags): (String, Value) = sqlx::query_as(
        "SELECT tag_state, tags FROM call_entries WHERE call_id = $1 AND kind = 'message'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (state.as_str(), tags["attempts"].as_i64()),
        ("pending", Some(1))
    );
    assert_eq!(
        super::memory::tagger::tag_once(&s).await.unwrap(),
        0,
        "leased"
    );
    // The third failure gives up.
    sqlx::query("UPDATE call_entries SET tagged_at = now() - interval '6 minutes', tags = '{\"attempts\": 2}' WHERE call_id = $1 AND kind = 'message'")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    super::memory::tagger::tag_once(&s).await.unwrap();
    let state: String = sqlx::query_scalar(
        "SELECT tag_state FROM call_entries WHERE call_id = $1 AND kind = 'message'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "error");

    // A caller out of credits waits.
    let s = memory_on(&pool, memory_fake()).await;
    post(
        &s,
        &w.alice.key,
        &format!("/v1/calls/{id}/end"),
        json!({ "agent_id": w.a }),
    )
    .await;
    ring(
        &s,
        &w,
        json!({ "memory": "call", "message": "Which venue?" }),
    )
    .await;
    sqlx::query(
        "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period)
         VALUES ($1, -100, date_trunc('month', now() AT TIME ZONE 'UTC')::date)",
    )
    .bind(w.alice.account)
    .execute(&pool)
    .await
    .unwrap();
    s.credit_cache.refresh(&pool, &s.credits).await.unwrap();
    assert_eq!(super::memory::tagger::tag_once(&s).await.unwrap(), 0);
}
