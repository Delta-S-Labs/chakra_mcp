//! Calling agents by name over MCP (MCP and discovery spec §6.1–6.2, §6.4,
//! §6.6): the caller rules, default agents, waiting, and the discovery
//! tools on `/mcp`.

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request};
use chakramcp_shared::config::SharedConfig;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::mcp::caller::{default_agent, Kind};

const SECRET: &str = "test-secret";

fn config(discovery: bool) -> SharedConfig {
    SharedConfig {
        database_url: "ignored".into(),
        jwt_secret: SECRET.into(),
        admin_email: None,
        survey_enabled: false,
        frontend_base_url: "https://web.example".into(),
        app_base_url: "http://localhost:8080".into(),
        relay_base_url: "https://relay.example".into(),
        discovery_v2_enabled: discovery,
        log_filter: "warn".into(),
    }
}

fn state(pool: &PgPool, discovery: bool) -> crate::state::RelayState {
    crate::state::RelayState::new(pool.clone(), config(discovery))
}

struct Person {
    id: Uuid,
    account: Uuid,
    token: String,
}

async fn person(pool: &PgPool, slug: &str) -> Person {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, 'x')",
    )
    .bind(id)
    .bind(format!("{id}@t.local"))
    .bind(format!("{slug} user"))
    .execute(pool)
    .await
    .unwrap();
    let account = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
         VALUES ($1, $2, $2, 'individual', $3)",
    )
    .bind(account)
    .bind(slug)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'owner')")
        .bind(Uuid::now_v7())
        .bind(account)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let token = chakramcp_shared::jwt::encode_jwt(
        &chakramcp_shared::jwt::UserClaims::new(id, format!("{id}@t.local"), false, 1),
        SECRET,
    )
    .unwrap();
    Person { id, account, token }
}

async fn agent(pool: &PgPool, account: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agents (id, account_id, slug, display_name, description, visibility)
         VALUES ($1, $2, $3, $3, 'An agent for testing things.', 'network')",
    )
    .bind(id)
    .bind(account)
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// A capability; `quota` makes it public with that monthly quota.
async fn capability(pool: &PgPool, agent: Uuid, name: &str, quota: Option<i32>) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agent_capabilities
            (id, agent_id, name, description, visibility, public_invoke, public_monthly_quota_per_agent)
         VALUES ($1, $2, $3, 'A capability.', 'network', $4, $5)",
    )
    .bind(id)
    .bind(agent)
    .bind(name)
    .bind(quota.is_some())
    .bind(quota)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn befriend_and_grant(pool: &PgPool, granter: Uuid, grantee: Uuid, capability: Uuid) -> Uuid {
    sqlx::query(
        "INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status) VALUES ($1, $2, $3, 'accepted')",
    )
    .bind(Uuid::now_v7())
    .bind(grantee)
    .bind(granter)
    .execute(pool)
    .await
    .unwrap();
    let grant = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO grants (id, granter_agent_id, grantee_agent_id, capability_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(grant)
    .bind(granter)
    .bind(grantee)
    .bind(capability)
    .execute(pool)
    .await
    .unwrap();
    grant
}

/// An API key for `user` with `scope` (`own` or `selected` over `agents`).
async fn scoped_key(pool: &PgPool, user: Uuid, scope: &str, agents: &[Uuid]) -> (String, Uuid) {
    let key_id = Uuid::now_v7();
    let key = format!("ck_scoped_{key_id}");
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix) VALUES ($1, $2, $3, 'Laptop Bot', 'ck_scoped')",
    )
    .bind(key_id)
    .bind(user)
    .bind(hex::encode(Sha256::digest(key.as_bytes())))
    .execute(pool)
    .await
    .unwrap();
    let scope_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO credential_scopes (id, cred_kind, cred_ref, user_id, agent_scope)
         VALUES ($1, 'api_key', $2, $3, $4)",
    )
    .bind(scope_id)
    .bind(key_id.to_string())
    .bind(user)
    .bind(scope)
    .execute(pool)
    .await
    .unwrap();
    for a in agents {
        sqlx::query(
            "INSERT INTO credential_scope_agents (credential_scope_id, agent_id) VALUES ($1, $2)",
        )
        .bind(scope_id)
        .bind(a)
        .execute(pool)
        .await
        .unwrap();
    }
    (key, key_id)
}

/// A JSON-RPC call to `/mcp`; returns `result`.
async fn rpc(pool: &PgPool, discovery: bool, token: &str, method: &str, params: Value) -> Value {
    let res = crate::router(state(pool, discovery))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    body["result"].clone()
}

async fn tool(pool: &PgPool, token: &str, name: &str, args: Value) -> Value {
    rpc(
        pool,
        true,
        token,
        "tools/call",
        json!({ "name": name, "arguments": args }),
    )
    .await
}

/// A tool result's value (it must have succeeded).
fn ok(result: &Value) -> Value {
    assert_eq!(result["isError"], false, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// A failed tool result's code.
fn code(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["structuredContent"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn grantee_of(pool: &PgPool, invocation: &Value) -> Uuid {
    let id: Uuid = invocation["invocation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    sqlx::query_scalar("SELECT grantee_agent_id FROM relay_invocations WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn user_of(p: &Person) -> AuthUser {
    AuthUser {
        user_id: p.id,
        email: String::new(),
        is_admin: false,
        api_key_id: None,
        minted_jti: None,
    }
}

// ─── The caller rules ─────────────────────────────────────────

/// By name, through a grant: called as the agent holding it.
#[sqlx::test(migrations = "../migrations")]
async fn by_name_through_a_grant(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    agent(&pool, alice.account, "helper").await;
    let planner = agent(&pool, alice.account, "planner").await;
    let venue = agent(&pool, bob.account, "venue").await;
    let hold = capability(&pool, venue, "hold", None).await;
    let grant = befriend_and_grant(&pool, venue, planner, hold).await;

    let out = ok(&tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "agent": "bob/venue", "capability": "hold", "input": { "room": "A" } }),
    )
    .await);
    assert_eq!(out["status"], "pending");
    assert_eq!(grantee_of(&pool, &out).await, planner);
    let used: Option<Uuid> =
        sqlx::query_scalar("SELECT grant_id FROM relay_invocations WHERE grantee_agent_id = $1")
            .bind(planner)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(used, Some(grant));

    // Without a grant, a friends-only capability says how to get access.
    capability(&pool, venue, "cancel", None).await;
    let refused = tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "agent": "bob/venue", "capability": "cancel", "input": {} }),
    )
    .await;
    assert_eq!(code(&refused), "chk.policy.grant_required");
    assert!(refused["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("propose_friendship"));

    // An unknown capability is "not found".
    let missing = tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "agent": "bob/venue", "capability": "nope", "input": {} }),
    )
    .await;
    assert_eq!(code(&missing), "not_found");
}

/// A public capability, unscoped: the personal agent, made once and reused,
/// and replaced after it's deleted.
#[sqlx::test(migrations = "../migrations")]
async fn public_by_name_calls_as_the_personal_agent(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    agent(&pool, alice.account, "helper").await;
    let venue = agent(&pool, bob.account, "venue").await;
    capability(&pool, venue, "check", Some(100)).await;
    let call = json!({ "agent": "bob/venue", "capability": "check", "input": {} });

    let first = ok(&tool(&pool, &alice.token, "invoke", call.clone()).await);
    let personal = grantee_of(&pool, &first).await;
    let row: (String, String, String, String, Uuid) = sqlx::query_as(
        "SELECT slug, display_name, visibility, accept_calls, account_id FROM agents WHERE id = $1",
    )
    .bind(personal)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (
            row.0.as_str(),
            row.1.as_str(),
            row.2.as_str(),
            row.3.as_str(),
            row.4
        ),
        (
            "personal",
            "alice user (personal)",
            "private",
            "nobody",
            alice.account
        )
    );
    let second = ok(&tool(&pool, &alice.token, "invoke", call.clone()).await);
    assert_eq!(grantee_of(&pool, &second).await, personal, "reused");

    // It's labelled on the agent API.
    let mine = ok(&tool(
        &pool,
        &alice.token,
        "get_agent",
        json!({ "agent_id": personal }),
    )
    .await);
    assert_eq!(mine["default_kind"], "personal");

    sqlx::query("UPDATE agents SET tombstoned_at = now() WHERE id = $1")
        .bind(personal)
        .execute(&pool)
        .await
        .unwrap();
    let third = ok(&tool(&pool, &alice.token, "invoke", call).await);
    let replacement = grantee_of(&pool, &third).await;
    assert_ne!(replacement, personal);
    let key_points_at: Uuid =
        sqlx::query_scalar("SELECT agent_id FROM default_agents WHERE key = $1")
            .bind(format!("user:{}", alice.id))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(key_points_at, replacement);
}

/// Concurrent first calls make one default agent; two users of one OAuth
/// client each get their own client agent.
#[sqlx::test(migrations = "../migrations")]
async fn default_agents_are_made_once_per_key(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let carol = person(&pool, "carol").await;
    let s = state(&pool, true);
    let a = user_of(&alice);
    let (one, two, three) = tokio::join!(
        default_agent(&s, &a, Kind::Personal),
        default_agent(&s, &a, Kind::Personal),
        default_agent(&s, &a, Kind::Personal),
    );
    let one = one.unwrap();
    assert_eq!(two.unwrap(), one);
    assert_eq!(three.unwrap(), one);
    let made: i64 = sqlx::query_scalar("SELECT count(*) FROM agents WHERE account_id = $1")
        .bind(alice.account)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(made, 1);

    sqlx::query(
        "INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris)
         VALUES ($1, 'claude-desktop', 'Claude Desktop', '{}')",
    )
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .unwrap();
    let client = Kind::OAuth("claude-desktop".into());
    let for_alice = default_agent(&s, &a, client.clone()).await.unwrap();
    let for_carol = default_agent(&s, &user_of(&carol), client).await.unwrap();
    assert_ne!(for_alice, for_carol);
    let slug: String = sqlx::query_scalar("SELECT slug FROM agents WHERE id = $1")
        .bind(for_alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(slug, "via-claude-desktop");
}

/// Scoped credentials: one agent in scope is the caller; several need `as`;
/// an `own` key with none gets a client agent that's then in its scope; an
/// `as` outside the scope is refused.
#[sqlx::test(migrations = "../migrations")]
async fn scoped_callers(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let first = agent(&pool, alice.account, "first").await;
    let second = agent(&pool, alice.account, "second").await;
    let venue = agent(&pool, bob.account, "venue").await;
    capability(&pool, venue, "check", Some(100)).await;
    let call = json!({ "agent": "bob/venue", "capability": "check", "input": {} });

    let (one, _) = scoped_key(&pool, alice.id, "selected", &[second]).await;
    let out = ok(&tool(&pool, &one, "invoke", call.clone()).await);
    assert_eq!(grantee_of(&pool, &out).await, second);

    let (both, _) = scoped_key(&pool, alice.id, "selected", &[first, second]).await;
    assert_eq!(
        code(&tool(&pool, &both, "invoke", call.clone()).await),
        "chk.mcp.as_required"
    );
    let mut with_as = call.clone();
    with_as["grantee_agent_id"] = json!(first);
    let out = ok(&tool(&pool, &both, "invoke", with_as.clone()).await);
    assert_eq!(grantee_of(&pool, &out).await, first);
    assert_eq!(
        code(&tool(&pool, &one, "invoke", with_as).await),
        "chk.auth.out_of_scope"
    );

    let (own, own_id) = scoped_key(&pool, alice.id, "own", &[]).await;
    let out = ok(&tool(&pool, &own, "invoke", call.clone()).await);
    let client_agent = grantee_of(&pool, &out).await;
    let (slug, by_key): (String, Option<Uuid>) =
        sqlx::query_as("SELECT slug, created_by_api_key_id FROM agents WHERE id = $1")
            .bind(client_agent)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((slug.as_str(), by_key), ("via-laptop-bot", Some(own_id)));
    // Now in the key's scope, so the next call uses it again.
    let out = ok(&tool(&pool, &own, "invoke", call).await);
    assert_eq!(grantee_of(&pool, &out).await, client_agent);
}

// ─── Waiting and refusals ─────────────────────────────────────

/// `wait_seconds` returns the finished invocation.
#[sqlx::test(migrations = "../migrations")]
async fn invoke_waits_for_the_result(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let venue = agent(&pool, bob.account, "venue").await;
    capability(&pool, venue, "check", Some(100)).await;

    let answerer = pool.clone();
    tokio::spawn(async move {
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let done = sqlx::query(
                "UPDATE relay_invocations SET status = 'succeeded', output_preview = '{\"free\": true}'
                  WHERE granter_agent_id = $1 AND status = 'pending'",
            )
            .bind(venue)
            .execute(&answerer)
            .await
            .unwrap();
            if done.rows_affected() > 0 {
                return;
            }
        }
    });
    let out = ok(&tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "agent": "bob/venue", "capability": "check", "input": {}, "wait_seconds": 10 }),
    )
    .await);
    assert_eq!(out["status"], "succeeded", "{out}");
    assert_eq!(out["output_preview"]["free"], true);
}

/// A used-up public quota is a tool error with its own code.
#[sqlx::test(migrations = "../migrations")]
async fn an_exhausted_quota_is_a_tool_error(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let venue = agent(&pool, bob.account, "venue").await;
    capability(&pool, venue, "check", Some(1)).await;
    let call = json!({ "agent": "bob/venue", "capability": "check", "input": {} });
    ok(&tool(&pool, &alice.token, "invoke", call.clone()).await);
    assert_eq!(
        code(&tool(&pool, &alice.token, "invoke", call).await),
        "monthly_quota_exhausted"
    );
}

/// The old shapes still work, and a mix is refused.
#[sqlx::test(migrations = "../migrations")]
async fn invoke_takes_one_way_of_naming(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let mine = agent(&pool, alice.account, "mine").await;
    let venue = agent(&pool, bob.account, "venue").await;
    let check = capability(&pool, venue, "check", Some(100)).await;
    let out = ok(&tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "capability_id": check, "grantee_agent_id": mine, "input": {} }),
    )
    .await);
    assert_eq!(grantee_of(&pool, &out).await, mine);
    let mixed = tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "capability_id": check, "agent": "bob/venue", "capability": "check", "input": {} }),
    )
    .await;
    assert_eq!(mixed["isError"], true);
    let no_caller = tool(
        &pool,
        &alice.token,
        "invoke",
        json!({ "capability_id": check, "input": {} }),
    )
    .await;
    assert_eq!(no_caller["isError"], true);
}

// ─── Discovery on /mcp ────────────────────────────────────────

/// Search and featured are listed only with the directory on; profiles and
/// `get_agent` by name always work.
#[sqlx::test(migrations = "../migrations")]
async fn discovery_tools(pool: PgPool) {
    let alice = person(&pool, "alice").await;
    let bob = person(&pool, "bob").await;
    let venue = agent(&pool, bob.account, "venue").await;
    capability(&pool, venue, "check", Some(100)).await;

    let names = |listed: &Value| -> Vec<String> {
        listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let on = names(&rpc(&pool, true, &alice.token, "tools/list", json!({})).await);
    let off = names(&rpc(&pool, false, &alice.token, "tools/list", json!({})).await);
    for tool_name in ["search_agents", "featured_agents", "get_agent_profile"] {
        assert!(
            on.iter().any(|n| n == tool_name),
            "{tool_name} listed when on"
        );
    }
    assert!(!off
        .iter()
        .any(|n| n == "search_agents" || n == "featured_agents"));
    assert!(off.iter().any(|n| n == "get_agent_profile"));

    let found = ok(&tool(
        &pool,
        &alice.token,
        "search_agents",
        json!({ "q": "venue" }),
    )
    .await);
    assert_eq!(found["agents"][0]["slug"], "venue");
    assert_eq!(found["agents"][0]["you"]["callable"][0]["via"], "public");

    let profile = ok(&tool(
        &pool,
        &alice.token,
        "get_agent_profile",
        json!({ "agent": "bob/venue" }),
    )
    .await);
    assert_eq!(profile["capabilities"][0]["name"], "check");
    assert!(profile.get("you").is_some());

    let fetched = ok(&tool(
        &pool,
        &alice.token,
        "get_agent",
        json!({ "agent": "bob/venue" }),
    )
    .await);
    assert_eq!(fetched["id"], venue.to_string());

    let network = ok(&tool(&pool, &alice.token, "list_network_agents", json!({})).await);
    let row = network
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["slug"] == "venue")
        .unwrap();
    for key in [
        "capabilities",
        "last_seen_at",
        "friend_count",
        "online",
        "is_mine",
        "capability_count",
    ] {
        assert!(row.get(key).is_some(), "{key}: {row}");
    }

    let instructions = rpc(&pool, true, &alice.token, "initialize", json!({})).await;
    assert!(instructions["instructions"]
        .as_str()
        .unwrap()
        .contains("search_agents"));
}

// ─── The public directory (spec §6.3) ─────────────────────────

mod directory {
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::extract::ConnectInfo;
    use axum::http::StatusCode;

    use super::*;

    fn directory_state(pool: &PgPool, on: bool, trust: bool) -> crate::state::RelayState {
        state(pool, true)
            .with_mcp(crate::mcp::McpConfig {
                agents_enabled: on,
                directory_per_ip_per_min: 2,
                trust_forwarded_for: trust,
                ..Default::default()
            })
            .with_rate_limiter(crate::limits::RateLimiter::Counting(Arc::new(Mutex::new(
                Default::default(),
            ))))
    }

    /// A directory request from `peer`, with optional extra headers.
    async fn send(
        s: &crate::state::RelayState,
        peer: &str,
        headers: &[(&str, &str)],
        method: &str,
        params: Value,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri("/mcp/directory")
            .header(header::CONTENT_TYPE, "application/json");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut req = req
            .body(Body::from(
                json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
                    .to_string(),
            ))
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let res = crate::router(s.clone()).oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&body).unwrap_or(Value::Null),
        )
    }

    /// Off, it's a 404; on, the tools answer without sign-in, show only
    /// network agents, never 401, and can't call anything.
    #[sqlx::test(migrations = "../migrations")]
    async fn serves_the_public_directory(pool: PgPool) {
        let bob = person(&pool, "bob").await;
        let venue = agent(&pool, bob.account, "venue").await;
        capability(&pool, venue, "check", Some(100)).await;
        let hidden = agent(&pool, bob.account, "hidden").await;
        sqlx::query("UPDATE agents SET visibility = 'private' WHERE id = $1")
            .bind(hidden)
            .execute(&pool)
            .await
            .unwrap();

        let (status, _, _) = send(
            &directory_state(&pool, false, false),
            "10.0.0.1:1",
            &[],
            "tools/list",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let s = crate::state::RelayState::new(pool.clone(), config(true)).with_mcp(
            crate::mcp::McpConfig {
                agents_enabled: true,
                ..Default::default()
            },
        );
        let (status, _, init) = send(&s, "10.0.0.1:1", &[], "initialize", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(init["result"]["serverInfo"]["name"], "chakramcp-directory");
        let (_, _, listed) = send(&s, "10.0.0.1:1", &[], "tools/list", json!({})).await;
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "search_agents",
                "get_agent_profile",
                "featured_agents",
                "list_reviews"
            ]
        );

        let call = |name: &str, args: Value| json!({ "name": name, "arguments": args });
        let (_, _, found) = send(
            &s,
            "10.0.0.1:1",
            &[],
            "tools/call",
            call("search_agents", json!({})),
        )
        .await;
        let found = ok(&found["result"]);
        let slugs: Vec<&str> = found["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["slug"].as_str().unwrap())
            .collect();
        assert_eq!(slugs, ["venue"]);
        assert!(found["agents"][0].get("you").is_none());

        let (_, _, profile) = send(
            &s,
            "10.0.0.1:1",
            &[],
            "tools/call",
            call("get_agent_profile", json!({ "agent": "bob/venue" })),
        )
        .await;
        assert_eq!(ok(&profile["result"])["capabilities"][0]["name"], "check");
        let (_, _, private) = send(
            &s,
            "10.0.0.1:1",
            &[],
            "tools/call",
            call("get_agent_profile", json!({ "agent": "bob/hidden" })),
        )
        .await;
        assert_eq!(private["result"]["isError"], true);
        let (_, _, reviews) = send(
            &s,
            "10.0.0.1:1",
            &[],
            "tools/call",
            call("list_reviews", json!({ "agent": "bob/venue" })),
        )
        .await;
        assert_eq!(ok(&reviews["result"])["count"], 0);
        let (_, _, callable) = send(
            &s,
            "10.0.0.1:1",
            &[],
            "tools/call",
            call("search_agents", json!({ "callable": true })),
        )
        .await;
        assert_eq!(callable["result"]["isError"], true);
        let (_, _, invoke) = send(
            &s,
            "10.0.0.1:1",
            &[],
            "tools/call",
            call("invoke", json!({})),
        )
        .await;
        assert!(invoke["error"].is_object(), "no invoke here: {invoke}");

        // A bad token changes nothing: no 401, no challenge.
        let (status, headers, _) = send(
            &s,
            "10.0.0.1:1",
            &[("authorization", "Bearer nope")],
            "tools/list",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!headers.contains_key(header::WWW_AUTHENTICATE));
    }

    /// Each IP gets its own bucket. Without TRUST_FORWARDED_FOR the header is
    /// ignored; with it, the last entry counts.
    #[sqlx::test(migrations = "../migrations")]
    async fn limits_each_client_ip(pool: PgPool) {
        let s = directory_state(&pool, true, false);
        for _ in 0..2 {
            let (status, _, _) = send(&s, "10.0.0.1:1", &[], "ping", json!({})).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, headers, body) = send(
            &s,
            "10.0.0.1:2",
            &[("x-forwarded-for", "192.0.2.9")],
            "ping",
            json!({}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "a forged header doesn't escape the limit"
        );
        assert_eq!(body["error"]["code"], "directory_rate_limited");
        assert!(headers.contains_key(header::RETRY_AFTER));
        let (status, _, _) = send(&s, "10.0.0.2:1", &[], "ping", json!({})).await;
        assert_eq!(status, StatusCode::OK, "another IP has its own bucket");

        // Behind a proxy: the proxy's address is shared, the last
        // X-Forwarded-For entry is the client.
        let s = directory_state(&pool, true, true);
        let proxy = "172.16.0.1:443";
        for _ in 0..2 {
            let (status, _, _) = send(
                &s,
                proxy,
                &[("x-forwarded-for", "forged, 198.51.100.7")],
                "ping",
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _, _) = send(
            &s,
            proxy,
            &[("x-forwarded-for", "198.51.100.7")],
            "ping",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        let (status, _, _) = send(
            &s,
            proxy,
            &[("x-forwarded-for", "198.51.100.8")],
            "ping",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}

// ─── Agents' own endpoints (spec §6.5) ────────────────────────

mod agent_endpoints {
    use axum::http::StatusCode;

    use super::*;

    fn endpoint_state(pool: &PgPool, on: bool) -> crate::state::RelayState {
        state(pool, true).with_mcp(crate::mcp::McpConfig {
            agents_enabled: on,
            tool_wait_secs: 2,
            ..Default::default()
        })
    }

    async fn post(
        s: &crate::state::RelayState,
        path: &str,
        token: Option<&str>,
        method: &str,
        params: Value,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let req = req
            .body(Body::from(
                json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
                    .to_string(),
            ))
            .unwrap();
        let res = crate::router(s.clone()).oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&body).unwrap_or(Value::Null),
        )
    }

    async fn call(
        s: &crate::state::RelayState,
        path: &str,
        token: &str,
        tool: &str,
        args: Value,
    ) -> Value {
        let (status, _, body) = post(
            s,
            path,
            Some(token),
            "tools/call",
            json!({ "name": tool, "arguments": args }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["result"].clone()
    }

    /// `structuredContent` fits the tools' output schema: a known status,
    /// and only its fields, of the right types.
    fn conforms(result: &Value) {
        let s = result["structuredContent"]
            .as_object()
            .expect("structuredContent");
        let status = s["status"].as_str().unwrap();
        assert!(
            [
                "succeeded",
                "pending",
                "in_progress",
                "awaiting_consent",
                "failed",
                "timeout",
                "refused"
            ]
            .contains(&status),
            "{status}"
        );
        for (k, v) in s {
            match k.as_str() {
                "status" | "invocation_id" | "code" | "error" => assert!(v.is_string(), "{k}: {v}"),
                "safety" | "peer" => assert!(v.is_object(), "{k}: {v}"),
                "output" => {}
                other => panic!("unexpected field {other}"),
            }
        }
    }

    /// Bob's venue agent: a public `check`, a friends-only `hold` (granted
    /// to Alice's planner), a friends-only `cancel`, a dotted `quote.ask`
    /// taking a string, and `book` that asks its owner first.
    struct Fixture {
        alice: Person,
        planner: Uuid,
        venue: Uuid,
    }

    async fn fixture(pool: &PgPool) -> Fixture {
        let alice = person(pool, "alice").await;
        let bob = person(pool, "bob").await;
        let planner = agent(pool, alice.account, "planner").await;
        let venue = agent(pool, bob.account, "venue").await;
        capability(pool, venue, "check", Some(100)).await;
        let hold = capability(pool, venue, "hold", None).await;
        befriend_and_grant(pool, venue, planner, hold).await;
        capability(pool, venue, "cancel", None).await;
        let quote = capability(pool, venue, "quote.ask", Some(100)).await;
        sqlx::query(
            "UPDATE agent_capabilities SET input_schema = '{\"type\":\"string\"}' WHERE id = $1",
        )
        .bind(quote)
        .execute(pool)
        .await
        .unwrap();
        let book = capability(pool, venue, "book", Some(100)).await;
        sqlx::query("UPDATE agent_capabilities SET consent_required = true WHERE id = $1")
            .bind(book)
            .execute(pool)
            .await
            .unwrap();
        Fixture {
            alice,
            planner,
            venue,
        }
    }

    /// Answer the venue's newest pending request.
    async fn answer(pool: &PgPool, venue: Uuid, status: &str, output: Value, error: Option<&str>) {
        let p = pool.clone();
        let status = status.to_owned();
        let error = error.map(str::to_owned);
        tokio::spawn(async move {
            for _ in 0..40 {
                tokio::time::sleep(Duration::from_millis(150)).await;
                let done = sqlx::query(
                    "UPDATE relay_invocations SET status = $2, output_preview = $3, error_message = $4
                      WHERE granter_agent_id = $1 AND status = 'pending'",
                )
                .bind(venue)
                .bind(&status)
                .bind(&output)
                .bind(&error)
                .execute(&p)
                .await
                .unwrap();
                if done.rows_affected() > 0 {
                    return;
                }
            }
        });
    }

    const PATH: &str = "/agents/bob/venue/mcp";

    /// Off is 404; no token or a bad one is 401 naming this endpoint's
    /// metadata, which is served; missing, private and deleted agents 404.
    #[sqlx::test(migrations = "../migrations")]
    async fn http_answers(pool: PgPool) {
        let f = fixture(&pool).await;
        let (status, _, _) = post(
            &endpoint_state(&pool, false),
            PATH,
            Some(&f.alice.token),
            "initialize",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let s = endpoint_state(&pool, true);
        for token in [None, Some("nope")] {
            let (status, headers, _) = post(&s, PATH, token, "initialize", json!({})).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(
                headers[header::WWW_AUTHENTICATE],
                r#"Bearer resource_metadata="https://relay.example/.well-known/oauth-protected-resource/agents/bob/venue/mcp""#
            );
        }
        let res = crate::router(s.clone())
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-protected-resource/agents/bob/venue/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let meta: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(
            meta["resource"],
            "https://relay.example/agents/bob/venue/mcp"
        );

        let (status, _, _) = post(
            &s,
            "/agents/bob/nobody/mcp",
            Some(&f.alice.token),
            "initialize",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let secret = agent(&pool, f.alice.account, "secret").await;
        sqlx::query("UPDATE agents SET visibility = 'private' WHERE id = $1")
            .bind(secret)
            .execute(&pool)
            .await
            .unwrap();
        let carol = person(&pool, "carol").await;
        let (status, _, _) = post(
            &s,
            "/agents/alice/secret/mcp",
            Some(&carol.token),
            "initialize",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "private to others");
        sqlx::query("UPDATE agents SET tombstoned_at = now() WHERE id = $1")
            .bind(f.venue)
            .execute(&pool)
            .await
            .unwrap();
        let (status, _, _) = post(&s, PATH, Some(&f.alice.token), "initialize", json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "deleted");
    }

    /// initialize names the agent; a push agent is a JSON-RPC error with its
    /// card.
    #[sqlx::test(migrations = "../migrations")]
    async fn initialize(pool: PgPool) {
        let f = fixture(&pool).await;
        let s = endpoint_state(&pool, true);
        let (_, _, body) = post(&s, PATH, Some(&f.alice.token), "initialize", json!({})).await;
        assert_eq!(body["result"]["serverInfo"]["name"], "venue");
        assert!(body["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("An agent for testing things."));

        sqlx::query("UPDATE agents SET mode = 'push', agent_card_url = 'https://push.example/.well-known/agent-card.json' WHERE id = $1")
            .bind(f.venue)
            .execute(&pool)
            .await
            .unwrap();
        let (_, _, body) = post(&s, PATH, Some(&f.alice.token), "initialize", json!({})).await;
        assert_eq!(body["error"]["data"]["code"], "chk.mcp.push_agent");
        assert!(body["error"]["data"]["a2a_card"]
            .as_str()
            .unwrap()
            .ends_with("/agents/bob/venue/.well-known/agent-card.json"));
    }

    /// tools/list offers what you can call now, with MCP-shaped names,
    /// wrapped non-object inputs and the output schema; a stranger sees only
    /// the public ones.
    #[sqlx::test(migrations = "../migrations")]
    async fn tools_list(pool: PgPool) {
        let f = fixture(&pool).await;
        let s = endpoint_state(&pool, true);
        let (_, _, body) = post(&s, PATH, Some(&f.alice.token), "tools/list", json!({})).await;
        let tools = body["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "book",
                "check",
                "hold",
                "quote_ask",
                "chakramcp_about",
                "chakramcp_get_result"
            ]
        );
        let quote = &tools[3];
        assert_eq!(quote["title"], "quote.ask");
        assert_eq!(quote["inputSchema"]["required"], json!(["input"]));
        assert_eq!(quote["outputSchema"]["required"], json!(["status"]));
        assert!(tools[0]["description"]
            .as_str()
            .unwrap()
            .contains("Asks its owner before running."));

        let carol = person(&pool, "carol").await;
        let (_, _, body) = post(&s, PATH, Some(&carol.token), "tools/list", json!({})).await;
        let names: Vec<&str> = body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "book",
                "check",
                "quote_ask",
                "chakramcp_about",
                "chakramcp_get_result"
            ]
        );
    }

    /// Each row of the result table, each fitting the output schema.
    #[sqlx::test(migrations = "../migrations")]
    async fn tool_results(pool: PgPool) {
        let f = fixture(&pool).await;
        let s = endpoint_state(&pool, true);
        let token = &f.alice.token;

        answer(&pool, f.venue, "succeeded", json!({ "free": true }), None).await;
        let done = call(&s, PATH, token, "hold", json!({ "room": "A" })).await;
        conforms(&done);
        assert_eq!(
            (
                done["isError"].clone(),
                done["structuredContent"]["status"].clone()
            ),
            (json!(false), json!("succeeded"))
        );
        assert_eq!(done["structuredContent"]["output"]["free"], true);
        let held_by: Uuid = sqlx::query_scalar("SELECT grantee_agent_id FROM relay_invocations WHERE granter_agent_id = $1 AND capability_name = 'hold'")
            .bind(f.venue)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(held_by, f.planner, "called as the grant's holder");

        let running = call(&s, PATH, token, "check", json!({})).await;
        conforms(&running);
        assert_eq!(running["structuredContent"]["status"], "pending");
        assert_eq!(running["isError"], false);
        let text = running["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("chakramcp_get_result"));
        // The venue agent has never pulled (waking pull agents §8).
        assert_eq!(
            running["structuredContent"]["peer"],
            json!({ "delivery": "pull", "online": false, "last_seen_at": null, "wake": false })
        );
        assert!(
            text.ends_with(
                "hasn't been seen in the last 30 days and has no wake URL, so it may not pick this up soon."
            ),
            "{text}"
        );
        let pending_id = running["structuredContent"]["invocation_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let held = call(&s, PATH, token, "book", json!({})).await;
        conforms(&held);
        assert_eq!(held["structuredContent"]["status"], "awaiting_consent");
        assert_eq!(held["structuredContent"]["peer"]["delivery"], "pull");

        sqlx::query("UPDATE relay_invocations SET status = 'failed', error_message = 'no rooms' WHERE id = $1")
            .bind(Uuid::parse_str(&pending_id).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let failed = call(
            &s,
            PATH,
            token,
            "chakramcp_get_result",
            json!({ "invocation_id": pending_id }),
        )
        .await;
        conforms(&failed);
        assert_eq!(
            (
                failed["isError"].clone(),
                failed["structuredContent"]["error"].clone()
            ),
            (json!(true), json!("no rooms"))
        );

        let refused = call(&s, PATH, token, "cancel", json!({})).await;
        conforms(&refused);
        assert_eq!(refused["structuredContent"]["status"], "refused");
        assert_eq!(
            refused["structuredContent"]["code"], "not_found",
            "not offered, so not a tool"
        );

        sqlx::query("UPDATE agent_capabilities SET public_monthly_quota_per_agent = 2 WHERE agent_id = $1 AND name = 'check'")
            .bind(f.venue)
            .execute(&pool)
            .await
            .unwrap();
        call(&s, PATH, token, "check", json!({})).await;
        let quota = call(&s, PATH, token, "check", json!({})).await;
        conforms(&quota);
        assert_eq!(
            quota["structuredContent"]["code"],
            "monthly_quota_exhausted"
        );
    }

    /// A wrapped input is unwrapped; `chakramcp_get_result` only shows your
    /// own calls to this agent; `chakramcp_about` says how to get access.
    #[sqlx::test(migrations = "../migrations")]
    async fn inputs_results_and_about(pool: PgPool) {
        let f = fixture(&pool).await;
        let s = endpoint_state(&pool, true);
        let quoted = call(
            &s,
            PATH,
            &f.alice.token,
            "quote_ask",
            json!({ "input": "two rooms" }),
        )
        .await;
        let id = quoted["structuredContent"]["invocation_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let input: Value =
            sqlx::query_scalar("SELECT input_preview FROM relay_invocations WHERE id = $1")
                .bind(Uuid::parse_str(&id).unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(input, json!("two rooms"));

        let carol = person(&pool, "carol").await;
        let other = call(
            &s,
            PATH,
            &carol.token,
            "chakramcp_get_result",
            json!({ "invocation_id": id }),
        )
        .await;
        assert_eq!(other["structuredContent"]["code"], "not_found");
        let mine = call(
            &s,
            PATH,
            &f.alice.token,
            "chakramcp_get_result",
            json!({ "invocation_id": id, "wait_seconds": 0 }),
        )
        .await;
        assert_eq!(mine["structuredContent"]["status"], "pending");

        let about = call(&s, PATH, &carol.token, "chakramcp_about", json!({})).await;
        let about = ok(&about);
        assert_eq!(
            about["how_to_call"]["mcp"],
            "https://relay.example/agents/bob/venue/mcp"
        );
        assert_eq!(about["not_callable_yet"], json!(["cancel", "hold"]));
        assert!(about["how_to_get_access"]
            .as_str()
            .unwrap()
            .contains("propose_friendship"));
    }

    /// `?as=` names the caller; outside the credential's scope it's refused
    /// at initialize.
    #[sqlx::test(migrations = "../migrations")]
    async fn as_names_the_caller(pool: PgPool) {
        let f = fixture(&pool).await;
        let s = endpoint_state(&pool, true);
        let helper = agent(&pool, f.alice.account, "helper").await;
        let path = format!("{PATH}?as=alice/helper");
        let out = call(&s, &path, &f.alice.token, "check", json!({})).await;
        let id =
            Uuid::parse_str(out["structuredContent"]["invocation_id"].as_str().unwrap()).unwrap();
        let by: Uuid =
            sqlx::query_scalar("SELECT grantee_agent_id FROM relay_invocations WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(by, helper);

        let (key, _) = scoped_key(&pool, f.alice.id, "selected", &[f.planner]).await;
        let (_, _, body) = post(&s, &path, Some(&key), "initialize", json!({})).await;
        assert_eq!(body["error"]["data"]["code"], "chk.auth.out_of_scope");
    }
}
