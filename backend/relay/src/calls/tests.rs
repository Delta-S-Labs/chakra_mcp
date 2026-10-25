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
