//! A credential's agent scope covers every action taken as an agent (MCP and
//! discovery spec §4.2): invoke, answering, A2A and MCP's reads, through the
//! in-memory scope cache.

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chakramcp_shared::config::SharedConfig;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::handlers::invoke::legacy_v01_contract_tests::{seed_demo, DemoFixture};
use crate::scope_cache::{Credential, ScopeCache};

fn config(discovery: bool) -> SharedConfig {
    SharedConfig {
        discovery_v2_enabled: discovery,
        ..crate::handlers::invoke::legacy_v01_contract_tests::config()
    }
}

async fn send(pool: &PgPool, cfg: SharedConfig, req: Request<Body>) -> (StatusCode, Value) {
    let app = crate::router(crate::state::RelayState::new(pool.clone(), cfg));
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn post(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn mcp(tool: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    })
}

async fn account_of(pool: &PgPool, agent: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT account_id FROM agents WHERE id = $1")
        .bind(agent)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn owner_of(pool: &PgPool, account: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT user_id FROM account_memberships WHERE account_id = $1")
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_agent(pool: &PgPool, account: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agents (id, account_id, slug, display_name, visibility)
         VALUES ($1, $2, $3, $3, 'network')",
    )
    .bind(id)
    .bind(account)
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// An API key for `user` whose scope is `selected` over `agents`.
async fn selected_key(pool: &PgPool, user: Uuid, agents: &[Uuid]) -> (String, Uuid) {
    let key_id = Uuid::now_v7();
    let key = format!("ck_scoped_{key_id}");
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
         VALUES ($1, $2, $3, 'scoped', 'ck_scoped')",
    )
    .bind(key_id)
    .bind(user)
    .bind(hex::encode(Sha256::digest(key.as_bytes())))
    .execute(pool)
    .await
    .unwrap();
    let scope = add_scope(pool, "api_key", &key_id.to_string(), user, "selected", None).await;
    for a in agents {
        select(pool, scope, *a).await;
    }
    (key, key_id)
}

async fn add_scope(
    pool: &PgPool,
    kind: &str,
    cred_ref: &str,
    user: Uuid,
    mode: &str,
    client: Option<&str>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO credential_scopes (id, cred_kind, cred_ref, client_id, user_id, agent_scope)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(kind)
    .bind(cred_ref)
    .bind(client)
    .bind(user)
    .bind(mode)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn select(pool: &PgPool, scope: Uuid, agent: Uuid) {
    sqlx::query(
        "INSERT INTO credential_scope_agents (credential_scope_id, agent_id) VALUES ($1, $2)",
    )
    .bind(scope)
    .bind(agent)
    .execute(pool)
    .await
    .unwrap();
}

fn invoke_body(f: &DemoFixture) -> Value {
    json!({ "grant_id": f.grant_id, "grantee_agent_id": f.grantee_agent_id, "input": {} })
}

#[sqlx::test(migrations = "../migrations")]
async fn a_scoped_key_cannot_invoke_or_answer_as_other_agents(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let bob_account = account_of(&pool, f.grantee_agent_id).await;
    let bob_other = add_agent(&pool, bob_account, "bob-other").await;
    let (bob_scoped, _) = selected_key(&pool, f.caller_user_id, &[bob_other]).await;

    // Invoking as bob-caller, outside the key's scope.
    let (status, body) = send(
        &pool,
        config(false),
        post("/v1/invoke", &bob_scoped, invoke_body(&f)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "chk.auth.out_of_scope");

    let (status, body) = send(
        &pool,
        config(false),
        post(
            "/mcp",
            &bob_scoped,
            mcp(
                "invoke",
                json!({ "grant_id": f.grant_id, "grantee_agent_id": f.grantee_agent_id, "input": {} }),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert_eq!(
        body["result"]["structuredContent"]["code"],
        "chk.auth.out_of_scope"
    );

    // The unscoped key still can.
    let (status, body) = send(
        &pool,
        config(false),
        post("/v1/invoke", &f.caller_token, invoke_body(&f)),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let invocation = body["invocation_id"].as_str().unwrap().to_owned();

    // Answering as alice-scheduler with a key scoped to another of Alice's agents.
    let alice_account = account_of(&pool, f.granter_agent_id).await;
    let alice = owner_of(&pool, alice_account).await;
    let alice_other = add_agent(&pool, alice_account, "alice-other").await;
    let (alice_scoped, _) = selected_key(&pool, alice, &[alice_other]).await;
    sqlx::query("UPDATE relay_invocations SET status = 'in_progress' WHERE id = $1::uuid")
        .bind(&invocation)
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = send(
        &pool,
        config(false),
        post(
            &format!("/v1/invocations/{invocation}/result"),
            &alice_scoped,
            json!({ "status": "succeeded", "output": {} }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "chk.auth.out_of_scope");

    let (_, body) = send(
        &pool,
        config(false),
        post(
            "/mcp",
            &alice_scoped,
            mcp(
                "respond",
                json!({ "invocation_id": invocation, "status": "succeeded", "output": {} }),
            ),
        ),
    )
    .await;
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert_eq!(
        body["result"]["structuredContent"]["code"],
        "chk.auth.out_of_scope"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn a2a_refuses_a_caller_agent_outside_the_scope(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let bob_account = account_of(&pool, f.grantee_agent_id).await;
    let bob_other = add_agent(&pool, bob_account, "bob-other").await;
    let (bob_scoped, _) = selected_key(&pool, f.caller_user_id, &[bob_other]).await;
    let alice_slug: String = sqlx::query_scalar(
        "SELECT acc.slug FROM accounts acc JOIN agents a ON a.account_id = acc.id WHERE a.id = $1",
    )
    .bind(f.granter_agent_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/agents/{alice_slug}/alice-scheduler/a2a/jsonrpc"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {bob_scoped}"))
        .header("X-ChakraMCP-Caller-Agent", f.grantee_agent_id.to_string())
        .header("X-ChakraMCP-Capability", f.capability_id.to_string())
        .body(Body::from(
            br#"{"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{}}"#.to_vec(),
        ))
        .unwrap();
    let (status, body) = send(&pool, config(true), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["data"]["code"], "chk.auth.out_of_scope");
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_reads_show_only_rows_in_scope(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let (_, body) = send(
        &pool,
        config(false),
        post("/v1/invoke", &f.caller_token, invoke_body(&f)),
    )
    .await;
    let invocation = body["invocation_id"].as_str().unwrap().to_owned();
    let bob_account = account_of(&pool, f.grantee_agent_id).await;
    let bob_other = add_agent(&pool, bob_account, "bob-other").await;
    let (bob_scoped, _) = selected_key(&pool, f.caller_user_id, &[bob_other]).await;

    for (tool, needle) in [
        ("list_grants", f.grant_id.to_string()),
        ("list_invocations", invocation.clone()),
        ("list_friendships", f.granter_agent_id.to_string()),
    ] {
        let (_, all) = send(
            &pool,
            config(false),
            post("/mcp", &f.caller_token, mcp(tool, json!({}))),
        )
        .await;
        assert!(all["result"].to_string().contains(&needle), "{tool}: {all}");
        let (_, scoped) = send(
            &pool,
            config(false),
            post("/mcp", &bob_scoped, mcp(tool, json!({}))),
        )
        .await;
        assert!(
            !scoped["result"].to_string().contains(&needle),
            "{tool}: {scoped}"
        );
    }

    let (_, body) = send(
        &pool,
        config(false),
        post(
            "/mcp",
            &bob_scoped,
            mcp("poll_invocation", json!({ "invocation_id": invocation })),
        ),
    )
    .await;
    assert_eq!(body["result"]["isError"], true, "{body}");
}

#[sqlx::test(migrations = "../migrations")]
async fn the_cache_answers_from_memory_and_rechecks_refusals(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let bob_account = account_of(&pool, f.grantee_agent_id).await;
    let bob_other = add_agent(&pool, bob_account, "bob-other").await;
    let (_, key_id) = selected_key(&pool, f.caller_user_id, &[]).await;
    let cred = Credential {
        user_id: f.caller_user_id,
        minted_jti: None,
        api_key_id: Some(key_id),
    };
    let cache = ScopeCache::default();
    assert!(!cache.allows(&pool, cred, bob_other).await.unwrap());

    // Added elsewhere (another replica, the app): a cached refusal looks again.
    let scope: Uuid = sqlx::query_scalar("SELECT id FROM credential_scopes WHERE cred_ref = $1")
        .bind(key_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    select(&pool, scope, bob_other).await;
    assert!(cache.allows(&pool, cred, bob_other).await.unwrap());

    // Now cached: answered with the database gone.
    pool.close().await;
    assert!(cache.allows(&pool, cred, bob_other).await.unwrap());
}

#[sqlx::test(migrations = "../migrations")]
async fn entries_expire(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let bob_account = account_of(&pool, f.grantee_agent_id).await;
    let bob_other = add_agent(&pool, bob_account, "bob-other").await;
    let (_, key_id) = selected_key(&pool, f.caller_user_id, &[bob_other]).await;
    let cred = Credential {
        user_id: f.caller_user_id,
        minted_jti: None,
        api_key_id: Some(key_id),
    };
    let lasting = ScopeCache::default();
    let fleeting = ScopeCache::with_ttl(Duration::ZERO);
    for cache in [&lasting, &fleeting] {
        assert!(crate::scope_cache::allows(
            &cache.in_scope(&pool, cred).await.unwrap(),
            bob_other
        ));
    }
    sqlx::query("DELETE FROM credential_scope_agents")
        .execute(&pool)
        .await
        .unwrap();
    let still = lasting.in_scope(&pool, cred).await.unwrap();
    let now = fleeting.in_scope(&pool, cred).await.unwrap();
    assert!(crate::scope_cache::allows(&still, bob_other));
    assert!(!crate::scope_cache::allows(&now, bob_other));
}

#[sqlx::test(migrations = "../migrations")]
async fn own_counts_only_your_accounts_and_tokens_keep_their_own_scopes(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let bob = f.caller_user_id;
    let bob_account = account_of(&pool, f.grantee_agent_id).await;
    let alice_account = account_of(&pool, f.granter_agent_id).await;
    sqlx::query(
        "INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris)
         VALUES ($1, 'shared-client', 'Shared', '{}')",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .unwrap();
    // The same client made an agent for Bob and one for Alice.
    let bobs = add_agent(&pool, bob_account, "bob-made").await;
    let alices = add_agent(&pool, alice_account, "alice-made").await;
    sqlx::query("UPDATE agents SET created_by_client_id = 'shared-client' WHERE id = ANY($1)")
        .bind(vec![bobs, alices])
        .execute(&pool)
        .await
        .unwrap();

    let own_jti = Uuid::now_v7();
    add_scope(
        &pool,
        "jwt",
        &own_jti.to_string(),
        bob,
        "own",
        Some("shared-client"),
    )
    .await;
    let cache = ScopeCache::default();
    let own = Credential {
        user_id: bob,
        minted_jti: Some(own_jti),
        api_key_id: None,
    };
    let scope = cache.in_scope(&pool, own).await.unwrap().unwrap();
    assert_eq!(scope.iter().copied().collect::<Vec<_>>(), vec![bobs]);

    // Another token of Bob's, with no scope row: all agents, its own entry.
    let other = Credential {
        user_id: bob,
        minted_jti: Some(Uuid::now_v7()),
        api_key_id: None,
    };
    assert!(cache
        .allows(&pool, other, f.grantee_agent_id)
        .await
        .unwrap());
    assert!(!cache.allows(&pool, own, f.grantee_agent_id).await.unwrap());
}
