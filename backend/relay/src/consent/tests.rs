//! Owner consent, gate 1, end to end through the router (spec §12). Uses
//! the v0.1.0 contract fixture: Alice owns the target agent (and its
//! capability), Bob calls it on a grant. Publishing can't turn consent on
//! yet (PR 2), so tests set the capability's columns directly.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::consent::visibility::HELD_MARKER;
use crate::handlers::invoke::legacy_v01_contract_tests::{config, seed_demo, DemoFixture};
use crate::state::RelayState;

const INPUT: &str = r#"{"duration_min": 30}"#;

// ─── Fixture helpers ─────────────────────────────────────

fn state(pool: &PgPool) -> RelayState {
    RelayState::new(pool.clone(), config())
}

/// Send a request through a fresh router: (status, JSON body or Null).
async fn send(
    pool: &PgPool,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_on(state(pool), method, uri, token, body).await
}

async fn send_on(
    state: RelayState,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(match body {
            Some(b) => Body::from(serde_json::to_vec(&b).unwrap()),
            None => Body::empty(),
        })
        .unwrap();
    let res = crate::router(state).oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Alice (the target's owner): her user and account.
async fn alice(pool: &PgPool, f: &DemoFixture) -> (Uuid, Uuid) {
    sqlx::query_as(
        "SELECT acc.owner_user_id, acc.id FROM agents a
           JOIN accounts acc ON acc.id = a.account_id WHERE a.id = $1",
    )
    .bind(f.granter_agent_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A web-session JWT for `user_id`: what the approval page sends.
fn session(user_id: Uuid) -> String {
    let claims = chakramcp_shared::jwt::UserClaims::new(user_id, "u@t.local".into(), false, 1);
    chakramcp_shared::jwt::encode_jwt(&claims, &config().jwt_secret).unwrap()
}

/// A JWT minted for a device pairing (an agent acting for `user_id`).
async fn delegated_session(pool: &PgPool, user_id: Uuid) -> String {
    let claims = chakramcp_shared::jwt::UserClaims::new(user_id, "u@t.local".into(), false, 1);
    sqlx::query(
        "INSERT INTO oauth_device_codes (id, device_code_hash, user_code, expires_at, minted_jti)
         VALUES ($1, $2, $3, now() + interval '10 minutes', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(format!("hash-{}", claims.jti))
    .bind(format!("U-{}", &claims.jti.simple().to_string()[..6]))
    .bind(claims.jti)
    .execute(pool)
    .await
    .unwrap();
    chakramcp_shared::jwt::encode_jwt(&claims, &config().jwt_secret).unwrap()
}

/// A `ck_` API key for `user_id`.
async fn api_key(pool: &PgPool, user_id: Uuid) -> String {
    let key = format!("ck_test_{}", Uuid::now_v7().simple());
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
         VALUES ($1, $2, $3, 'test', 'ck_test')",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(hex::encode(Sha256::digest(key.as_bytes())))
    .execute(pool)
    .await
    .unwrap();
    key
}

/// A new user in `account_id` with `role`.
async fn member(pool: &PgPool, account_id: Uuid, role: &str) -> Uuid {
    let user_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash)
         VALUES ($1, $2, 'Carol', 'x')",
    )
    .bind(user_id)
    .bind(format!("carol-{user_id}@t.local"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(account_id)
    .bind(user_id)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    user_id
}

/// Make the fixture's capability ask first, allowing consents up to
/// `max_validity_secs`.
async fn ask_first(pool: &PgPool, f: &DemoFixture, max_validity_secs: i32) {
    sqlx::query(
        "UPDATE agent_capabilities
            SET consent_required = true, consent_max_validity_secs = $2
          WHERE id = $1",
    )
    .bind(f.capability_id)
    .bind(max_validity_secs)
    .execute(pool)
    .await
    .unwrap();
}

/// Bob invokes on his grant, optionally proposing `terms`.
async fn invoke(pool: &PgPool, f: &DemoFixture, terms: Option<Value>) -> (StatusCode, Value) {
    let mut body = json!({
        "grant_id": f.grant_id,
        "grantee_agent_id": f.grantee_agent_id,
        "input": serde_json::from_str::<Value>(INPUT).unwrap(),
    });
    if let Some(t) = terms {
        body["consent_terms"] = t;
    }
    send(pool, "POST", "/v1/invoke", &f.caller_token, Some(body)).await
}

fn uuid(v: &Value) -> Uuid {
    v.as_str().unwrap().parse().unwrap()
}

async fn invocation_status(pool: &PgPool, id: Uuid) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, error_message FROM relay_invocations WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn request_status(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM consent_requests WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn charged(pool: &PgPool, invocation_id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM credit_charge_queue WHERE invocation_id = $1)")
        .bind(invocation_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Hold one request: (invocation id, consent request id).
async fn hold(pool: &PgPool, f: &DemoFixture, terms: Option<Value>) -> (Uuid, Uuid) {
    let (status, body) = invoke(pool, f, terms).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "awaiting_consent", "{body}");
    (
        uuid(&body["invocation_id"]),
        uuid(&body["consent_request_id"]),
    )
}

// ─── Holding and approving ───────────────────────────────

#[sqlx::test(migrations = "../migrations")]
async fn held_until_approved_then_delivered_with_a_token(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let alice_key = api_key(&pool, alice_user).await;

    let (inv, req) = hold(&pool, &f, None).await;
    assert_eq!(invocation_status(&pool, inv).await.0, "awaiting_consent");
    assert!(!charged(&pool, inv).await, "held requests aren't charged");
    let notices: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_notices
          WHERE agent_id = $1 AND kind = 'consent_requested'
            AND payload->>'consent_request_id' = $2",
    )
    .bind(f.granter_agent_id)
    .bind(req.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(notices, 1);

    // The inbox never delivers it, and the target side can't read it.
    let inbox = format!("/v1/inbox?agent_id={}", f.granter_agent_id);
    let (_, pulled) = send(&pool, "GET", &inbox, &alice_key, None).await;
    assert_eq!(pulled, json!([]));
    let read = format!("/v1/invocations/{inv}");
    let (_, as_target) = send(&pool, "GET", &read, &alice_key, None).await;
    assert_eq!(as_target["input_preview"], json!({ HELD_MARKER: true }));
    assert_eq!(as_target["status"], "awaiting_consent");
    // The requester still sees what it sent.
    let (_, as_requester) = send(&pool, "GET", &read, &f.caller_token, None).await;
    assert_eq!(as_requester["input_preview"], json!({"duration_min": 30}));

    // The approval page shows the input to the approver.
    let page = format!("/v1/approvals/{req}");
    let (status, view) = send(&pool, "GET", &page, &session(alice_user), None).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["may_decide"], true);
    assert_eq!(view["input"], json!({"duration_min": 30}));
    assert_eq!(view["capability"]["name"], "propose_slots");
    assert_eq!(view["tier"], "friend");
    let (_, waiting) = send(&pool, "GET", "/v1/approvals", &session(alice_user), None).await;
    assert_eq!(waiting.as_array().unwrap().len(), 1);

    let (status, approved) = send(
        &pool,
        "POST",
        &format!("{page}/approve"),
        &session(alice_user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "approved");
    assert_eq!(invocation_status(&pool, inv).await.0, "pending");
    assert!(charged(&pool, inv).await, "charged at approval");

    // Delivered with a token bound to the input.
    let (_, pulled) = send(&pool, "GET", &inbox, &alice_key, None).await;
    let item = &pulled[0];
    assert_eq!(item["input_preview"], json!({"duration_min": 30}));
    let token = item["consent"]["token"].as_str().unwrap().to_owned();
    let keys = crate::agent_card::keys::KeyStore::new(pool.clone())
        .jwks_keys()
        .await
        .unwrap();
    let claims = crate::consent::token::verify(
        &token,
        &keys,
        Utc::now(),
        Some(&json!({"duration_min": 30})),
    )
    .unwrap();
    assert_eq!(claims.jti, req.to_string());
    assert_eq!(claims.approver, alice_user.to_string());
    assert_eq!(claims.iss, "http://localhost:8090");
    assert!(crate::consent::token::verify(&token, &keys, Utc::now(), Some(&json!({}))).is_err());
    // Stored, so later reads return the same token.
    let (_, later) = send(&pool, "GET", &read, &f.caller_token, None).await;
    assert_eq!(later["consent"]["token"], json!(token));
}

#[sqlx::test(migrations = "../migrations")]
async fn only_a_web_session_of_an_approver_may_decide(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (_, req) = hold(&pool, &f, None).await;
    let approve = format!("/v1/approvals/{req}/approve");

    let key = api_key(&pool, alice_user).await;
    assert_eq!(
        send(&pool, "POST", &approve, &key, None).await.0,
        StatusCode::FORBIDDEN
    );
    let delegated = delegated_session(&pool, alice_user).await;
    assert_eq!(
        send(&pool, "POST", &approve, &delegated, None).await.0,
        StatusCode::FORBIDDEN
    );
    // Bob isn't in Alice's account: not even told it exists.
    assert_eq!(
        send(&pool, "POST", &approve, &session(f.caller_user_id), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(request_status(&pool, req).await, "pending");
}

#[sqlx::test(migrations = "../migrations")]
async fn approvers_fall_back_to_owners_when_none_listed_remain(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, alice_account) = alice(&pool, &f).await;
    let carol = member(&pool, alice_account, "member").await;

    // Listed: Carol only. Alice (owner) may not decide; Carol may.
    sqlx::query(
        "UPDATE agent_capabilities SET consent_approvers = ARRAY[$2]::uuid[] WHERE id = $1",
    )
    .bind(f.capability_id)
    .bind(carol)
    .execute(&pool)
    .await
    .unwrap();
    let (_, req) = hold(&pool, &f, None).await;
    let approve = format!("/v1/approvals/{req}/approve");
    assert_eq!(
        send(&pool, "POST", &approve, &session(alice_user), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (_, view) = send(
        &pool,
        "GET",
        &format!("/v1/approvals/{req}"),
        &session(alice_user),
        None,
    )
    .await;
    assert_eq!(view["may_decide"], false);
    assert!(
        view.get("input").is_none(),
        "non-approvers don't see the input"
    );
    assert_eq!(
        send(&pool, "POST", &approve, &session(carol), None).await.0,
        StatusCode::OK
    );

    // Listed: someone who has left. The owner decides.
    sqlx::query(
        "UPDATE agent_capabilities SET consent_approvers = ARRAY[gen_random_uuid()] WHERE id = $1",
    )
    .bind(f.capability_id)
    .execute(&pool)
    .await
    .unwrap();
    let (_, req) = hold(&pool, &f, None).await;
    assert_eq!(
        send(
            &pool,
            "POST",
            &format!("/v1/approvals/{req}/approve"),
            &session(alice_user),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn deny_rejects_without_charging(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (inv, req) = hold(&pool, &f, None).await;

    let (status, _) = send(
        &pool,
        "POST",
        &format!("/v1/approvals/{req}/deny"),
        &session(alice_user),
        Some(json!({"note": "not this week"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(request_status(&pool, req).await, "denied");
    assert_eq!(
        invocation_status(&pool, inv).await,
        (
            "rejected".into(),
            Some("The owner declined: not this week".into())
        )
    );
    assert!(!charged(&pool, inv).await);
    // Decided requests can't be decided again.
    assert_eq!(
        send(
            &pool,
            "POST",
            &format!("/v1/approvals/{req}/approve"),
            &session(alice_user),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // The requester sees the outcome, never the input.
    let (status, seen) = send(
        &pool,
        "GET",
        &format!("/v1/consent-requests/{req}"),
        &f.caller_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen["status"], "denied");
    assert_eq!(seen["decision_note"], "not this week");
}

// ─── Standing consents ───────────────────────────────────

#[sqlx::test(migrations = "../migrations")]
async fn a_standing_consent_covers_requests_up_to_its_limit(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 7 * 86400).await;
    let (alice_user, _) = alice(&pool, &f).await;
    // Bob asks for a month; the publisher's limit caps it at 7 days.
    let (_, req) = hold(
        &pool,
        &f,
        Some(json!({"valid_for": "30d", "max_requests": 2})),
    )
    .await;
    let terms: Value = sqlx::query_scalar("SELECT terms FROM consent_requests WHERE id = $1")
        .bind(req)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(terms["valid_for"], "7d");

    let (status, approved) = send(
        &pool,
        "POST",
        &format!("/v1/approvals/{req}/approve"),
        &session(alice_user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let consent_id = uuid(&approved["consent_id"]);

    // The second request is covered (the approved one used the first).
    let (status, covered) = invoke(&pool, &f, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(covered["status"], "pending", "{covered}");
    let covered_inv = uuid(&covered["invocation_id"]);
    assert!(charged(&pool, covered_inv).await);
    assert_eq!(
        request_status(&pool, uuid(&covered["consent_request_id"])).await,
        "approved"
    );
    let used: i32 = sqlx::query_scalar("SELECT used FROM consents WHERE id = $1")
        .bind(consent_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(used, 2);

    // The third is held: the consent is used up.
    hold(&pool, &f, None).await;

    // A covered request's token names the consent and its approver.
    let alice_key = api_key(&pool, alice_user).await;
    let (_, pulled) = send(
        &pool,
        "GET",
        &format!("/v1/inbox?agent_id={}", f.granter_agent_id),
        &alice_key,
        None,
    )
    .await;
    let covered_item = pulled
        .as_array()
        .unwrap()
        .iter()
        .find(|i| uuid(&i["id"]) == covered_inv)
        .unwrap();
    assert_eq!(covered_item["consent"]["consent_id"], json!(consent_id));
    assert!(covered_item["consent"]["token"].is_string());
}

#[sqlx::test(migrations = "../migrations")]
async fn approvers_can_narrow_but_never_widen(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 30 * 86400).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (_, req) = hold(
        &pool,
        &f,
        Some(json!({"valid_for": "7d", "max_requests": 5})),
    )
    .await;
    let approve = format!("/v1/approvals/{req}/approve");

    for wider in [json!({"valid_for": "8d"}), json!({"max_requests": 6})] {
        let (status, _) = send(&pool, "POST", &approve, &session(alice_user), Some(wider)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, approved) = send(
        &pool,
        "POST",
        &approve,
        &session(alice_user),
        Some(json!({"valid_for": "1d", "max_requests": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["approved_terms"]["valid_for"], "1d");
    // max_requests 1 was spent on the approved request itself.
    hold(&pool, &f, None).await;
}

#[sqlx::test(migrations = "../migrations")]
async fn two_callers_racing_for_the_last_use_get_one_each_way(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 7 * 86400).await;
    let (alice_user, _) = alice(&pool, &f).await;
    sqlx::query(
        "INSERT INTO consents (id, capability_id, granter_agent_id, grantee_agent_id,
                               valid_from, valid_until, max_requests, used, approved_by_user_id)
         VALUES ($1, $2, $3, $4, now(), now() + interval '1 day', 2, 1, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(f.capability_id)
    .bind(f.granter_agent_id)
    .bind(f.grantee_agent_id)
    .bind(alice_user)
    .execute(&pool)
    .await
    .unwrap();

    let (a, b) = tokio::join!(invoke(&pool, &f, None), invoke(&pool, &f, None));
    let mut statuses = [a.1["status"].clone(), b.1["status"].clone()];
    statuses.sort_by_key(|s| s.to_string());
    assert_eq!(statuses, [json!("awaiting_consent"), json!("pending")]);
}

#[sqlx::test(migrations = "../migrations")]
async fn a_revoked_consent_stops_covering(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 7 * 86400).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (_, req) = hold(&pool, &f, Some(json!({"valid_for": "7d"}))).await;
    let (_, approved) = send(
        &pool,
        "POST",
        &format!("/v1/approvals/{req}/approve"),
        &session(alice_user),
        None,
    )
    .await;
    let consent_id = uuid(&approved["consent_id"]);
    let (_, active) = send(
        &pool,
        "GET",
        "/v1/approvals?state=active",
        &session(alice_user),
        None,
    )
    .await;
    assert_eq!(active[0]["id"], json!(consent_id));

    let revoke = format!("/v1/consents/{consent_id}/revoke");
    let key = api_key(&pool, alice_user).await;
    assert_eq!(
        send(&pool, "POST", &revoke, &key, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&pool, "POST", &revoke, &session(alice_user), None)
            .await
            .0,
        StatusCode::OK
    );
    hold(&pool, &f, None).await;
}

// ─── What approval re-checks, and what cancels ───────────

#[sqlx::test(migrations = "../migrations")]
async fn approving_after_the_grant_is_revoked_cancels(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (inv, req) = hold(&pool, &f, None).await;
    // Revoked behind the cascade's back, so approval has to notice.
    sqlx::query("UPDATE grants SET status = 'revoked' WHERE id = $1")
        .bind(f.grant_id)
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = send(
        &pool,
        "POST",
        &format!("/v1/approvals/{req}/approve"),
        &session(alice_user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(request_status(&pool, req).await, "cancelled");
    assert_eq!(
        invocation_status(&pool, inv).await,
        (
            "rejected".into(),
            Some("No longer authorized: the grant was revoked".into())
        )
    );
    assert!(!charged(&pool, inv).await);
}

#[sqlx::test(migrations = "../migrations")]
async fn revoking_the_grant_cancels_held_requests(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (inv, req) = hold(&pool, &f, None).await;
    let (status, _) = send(
        &pool,
        "POST",
        &format!("/v1/grants/{}/revoke", f.grant_id),
        &session(alice_user),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(invocation_status(&pool, inv).await.0, "rejected");
    assert_eq!(request_status(&pool, req).await, "cancelled");
}

#[sqlx::test(migrations = "../migrations")]
async fn deleting_the_capability_or_agent_cancels_held_requests(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 7 * 86400).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (inv, req) = hold(&pool, &f, None).await;
    let (status, _) = send(
        &pool,
        "DELETE",
        &format!(
            "/v1/agents/{}/capabilities/{}",
            f.granter_agent_id, f.capability_id
        ),
        &session(alice_user),
        None,
    )
    .await;
    assert!(status.is_success(), "{status}");
    assert_eq!(
        invocation_status(&pool, inv).await,
        ("rejected".into(), Some("Capability deleted".into()))
    );
    assert_eq!(request_status(&pool, req).await, "cancelled");

    // An agent: requests from Bob's agent to a second capability.
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (inv, req) = hold(&pool, &f, None).await;
    let (status, _) = send(
        &pool,
        "DELETE",
        &format!("/v1/agents/{}", f.grantee_agent_id),
        &session(f.caller_user_id),
        None,
    )
    .await;
    assert!(status.is_success(), "{status}");
    assert_eq!(
        invocation_status(&pool, inv).await,
        ("rejected".into(), Some("Agent deleted".into()))
    );
    assert_eq!(request_status(&pool, req).await, "cancelled");
}

#[sqlx::test(migrations = "../migrations")]
async fn undecided_requests_expire(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (inv, req) = hold(&pool, &f, None).await;
    assert_eq!(
        crate::consent::expiry::expire_once(&pool).await.unwrap(),
        (0, 0)
    );
    sqlx::query(
        "UPDATE consent_requests SET expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(req)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        crate::consent::expiry::expire_once(&pool).await.unwrap(),
        (1, 1)
    );
    assert_eq!(request_status(&pool, req).await, "expired");
    assert_eq!(
        invocation_status(&pool, inv).await,
        ("rejected".into(), Some("Consent request expired".into()))
    );
    assert!(!charged(&pool, inv).await);
}

// ─── Every path holds; reads hide; limits ────────────────

#[sqlx::test(migrations = "../migrations")]
async fn the_public_path_holds_too(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    sqlx::query(
        "UPDATE agent_capabilities
            SET public_invoke = true, public_monthly_quota_per_agent = 10
          WHERE id = $1",
    )
    .bind(f.capability_id)
    .execute(&pool)
    .await
    .unwrap();
    let (status, body) = send(
        &pool,
        "POST",
        "/v1/invoke",
        &f.caller_token,
        Some(json!({
            "capability_id": f.capability_id,
            "grantee_agent_id": f.grantee_agent_id,
            "input": {"q": 1},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "awaiting_consent");

    // Approving a public request re-checks that it's still public.
    let (alice_user, _) = alice(&pool, &f).await;
    sqlx::query("UPDATE agent_capabilities SET public_invoke = false, public_monthly_quota_per_agent = NULL WHERE id = $1")
        .bind(f.capability_id)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = send(
        &pool,
        "POST",
        &format!(
            "/v1/approvals/{}/approve",
            body["consent_request_id"].as_str().unwrap()
        ),
        &session(alice_user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_invoke_holds_and_mcp_reads_hide_the_input(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let alice_key = api_key(&pool, alice_user).await;
    let call = |token: String, name: &'static str, arguments: Value| {
        let pool = pool.clone();
        async move {
            let (_, body) = send(
                &pool,
                "POST",
                "/mcp",
                &token,
                Some(json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": name, "arguments": arguments },
                })),
            )
            .await;
            let result = &body["result"];
            result.get("structuredContent").cloned().unwrap_or_else(|| {
                serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
            })
        }
    };

    let held = call(
        f.caller_token.clone(),
        "invoke",
        json!({
            "grant_id": f.grant_id,
            "grantee_agent_id": f.grantee_agent_id,
            "input": {"secret": "plans"},
        }),
    )
    .await;
    assert_eq!(held["status"], "awaiting_consent", "{held}");
    let inv = held["invocation_id"].clone();

    let listed = call(
        alice_key.clone(),
        "list_invocations",
        json!({"status": "awaiting_consent"}),
    )
    .await;
    assert_eq!(listed[0]["input_preview"], json!({ HELD_MARKER: true }));
    let polled = call(
        alice_key.clone(),
        "poll_invocation",
        json!({"invocation_id": inv}),
    )
    .await;
    assert_eq!(polled["input_preview"], json!({ HELD_MARKER: true }));
    let pulled = call(
        alice_key,
        "pull_inbox",
        json!({"agent_id": f.granter_agent_id}),
    )
    .await;
    assert_eq!(pulled, json!([]));
    let mine = call(
        f.caller_token.clone(),
        "poll_invocation",
        json!({"invocation_id": inv}),
    )
    .await;
    assert_eq!(mine["input_preview"], json!({"secret": "plans"}));
}

#[sqlx::test(migrations = "../migrations")]
async fn held_input_is_hidden_even_when_one_user_is_on_both_sides(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (_, alice_account) = alice(&pool, &f).await;
    // Bob joins Alice's account: he's now on the target side too.
    sqlx::query(
        "INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'member')",
    )
    .bind(Uuid::now_v7())
    .bind(alice_account)
    .bind(f.caller_user_id)
    .execute(&pool)
    .await
    .unwrap();
    let (inv, _) = hold(&pool, &f, None).await;
    let (_, outbound) = send(
        &pool,
        "GET",
        "/v1/invocations?direction=outbound",
        &f.caller_token,
        None,
    )
    .await;
    let row = outbound
        .as_array()
        .unwrap()
        .iter()
        .find(|r| uuid(&r["id"]) == inv)
        .unwrap();
    assert_eq!(row["input_preview"], json!({ HELD_MARKER: true }));
}

#[sqlx::test(migrations = "../migrations")]
async fn inputs_must_fit_whole_and_terms_must_be_in_range(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (status, body) = send(
        &pool,
        "POST",
        "/v1/invoke",
        &f.caller_token,
        Some(json!({
            "grant_id": f.grant_id,
            "grantee_agent_id": f.grantee_agent_id,
            "input": {"blob": "x".repeat(17 * 1024)},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "chk.request.too_large");

    let (status, _) = invoke(
        &pool,
        &f,
        Some(json!({"valid_for": "7d", "max_requests": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let none: i64 = sqlx::query_scalar("SELECT count(*) FROM relay_invocations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(none, 0, "nothing is written for refused input or terms");
}

#[sqlx::test(migrations = "../migrations")]
async fn capabilities_without_consent_are_unchanged(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let (status, body) = invoke(&pool, &f, Some(json!({"valid_for": "7d"}))).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "pending");
    assert!(body.get("consent_request_id").is_none());
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM consent_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(requests, 0);
}

#[sqlx::test(migrations = "../migrations")]
async fn a_held_request_doesnt_make_the_requester_a_reviewer(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    hold(&pool, &f, None).await;
    let (status, body) = send(
        &pool,
        "GET",
        &format!("/v1/agents/{}/reviews/eligibility", f.granter_agent_id),
        &f.caller_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["eligible"], json!([]));
}

#[sqlx::test(migrations = "../migrations")]
async fn an_agent_that_asks_first_cant_become_push_mode(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let (status, body) = send(
        &pool,
        "PATCH",
        &format!("/v1/agents/{}", f.granter_agent_id),
        &session(alice_user),
        Some(json!({"agent_card_url": "https://alice.example/.well-known/agent-card.json"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("ask its owner"));
}

// ─── Notices and status ──────────────────────────────────

/// One MCP tool call's result payload.
async fn mcp(pool: &PgPool, token: &str, name: &str, arguments: Value) -> Value {
    let (_, body) = send(
        pool,
        "POST",
        "/mcp",
        token,
        Some(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        })),
    )
    .await;
    let result = &body["result"];
    assert_eq!(result["isError"], json!(false), "{body}");
    result.get("structuredContent").cloned().unwrap_or_else(|| {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    })
}

#[sqlx::test(migrations = "../migrations")]
async fn notices_carry_the_link_once_and_never_the_input(pool: PgPool) {
    let f = seed_demo(&pool).await;
    ask_first(&pool, &f, 0).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let alice_key = api_key(&pool, alice_user).await;
    let (_, req) = hold(&pool, &f, Some(json!({"purpose": "weekly sync"}))).await;
    let path = format!("/v1/inbox/notices?agent_id={}", f.granter_agent_id);

    // Only the target's side may pull them.
    assert_eq!(
        send(&pool, "GET", &path, &f.caller_token, None).await.0,
        StatusCode::FORBIDDEN
    );

    let (status, notices) = send(&pool, "GET", &path, &alice_key, None).await;
    assert_eq!(status, StatusCode::OK, "{notices}");
    let notice = &notices[0];
    assert_eq!(notice["kind"], "consent_requested");
    let request = &notice["consent_request"];
    assert_eq!(request["id"], json!(req));
    assert_eq!(request["capability_name"], "propose_slots");
    assert_eq!(request["requester"]["agent_display_name"], "Bob Caller");
    assert_eq!(request["terms"]["purpose"], "weekly sync");
    assert_eq!(
        request["approve_url"],
        format!("http://localhost:3000/app/approvals/{req}")
    );
    assert!(
        !notices.to_string().contains("duration_min"),
        "never the input"
    );
    // Delivered once.
    assert_eq!(
        send(&pool, "GET", &path, &alice_key, None).await.1,
        json!([])
    );

    // MCP: the same notices, and the request's status for the caller.
    let (_, req2) = hold(&pool, &f, None).await;
    let pulled = mcp(
        &pool,
        &alice_key,
        "pull_notices",
        json!({"agent_id": f.granter_agent_id}),
    )
    .await;
    assert_eq!(pulled[0]["consent_request"]["id"], json!(req2));
    let status = mcp(
        &pool,
        &f.caller_token,
        "consent_status",
        json!({"consent_request_id": req2}),
    )
    .await;
    assert_eq!(status["status"], "pending");
    assert!(status.get("input").is_none());
}

// ─── Publishing consent settings ─────────────────────────

#[sqlx::test(migrations = "../migrations")]
async fn publishing_sets_and_checks_consent(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let (alice_user, alice_account) = alice(&pool, &f).await;
    let carol = member(&pool, alice_account, "member").await;
    let carol_email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(carol)
        .fetch_one(&pool)
        .await
        .unwrap();
    let key = api_key(&pool, alice_user).await;
    let caps = format!("/v1/agents/{}/capabilities", f.granter_agent_id);

    let (status, cap) = send(
        &pool,
        "POST",
        &caps,
        &key,
        Some(json!({
            "name": "pay_invoice",
            "consent": {"required": true, "approvers": [carol_email], "max_validity": "7d", "wait": "2h"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cap}");
    assert_eq!(
        cap["consent"],
        json!({"required": true, "approvers": [carol], "max_validity": "7d", "wait": "2h"})
    );
    assert_eq!(cap["semantics"], "autonomous");

    // Outsiders see that it asks first, not who decides.
    let (_, listed) = send(&pool, "GET", &caps, &f.caller_token, None).await;
    let seen = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "pay_invoice")
        .unwrap()
        .clone();
    assert_eq!(seen["consent"]["required"], true);
    assert!(seen["consent"].get("approvers").is_none());

    // Bad values are refused.
    for bad in [
        json!({"approvers": ["nobody@example.test"]}),
        json!({"max_validity": "2d"}),
        json!({"wait": "1m"}),
    ] {
        let (status, body) = send(
            &pool,
            "POST",
            &caps,
            &key,
            Some(json!({"name": "other", "consent": bad})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    // Update: absent fields keep their value; turning it off works.
    let (status, updated) = send(
        &pool,
        "PATCH",
        &format!("{caps}/{}", cap["id"].as_str().unwrap()),
        &key,
        Some(json!({"consent": {"required": false}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["consent"]["required"], false);
    assert_eq!(updated["consent"]["wait"], "2h");

    // A push-mode agent can't ask first.
    sqlx::query(
        "UPDATE agents SET mode = 'push',
                agent_card_url = 'https://alice.example/.well-known/agent-card.json'
          WHERE id = $1",
    )
    .bind(f.granter_agent_id)
    .execute(&pool)
    .await
    .unwrap();
    let (status, _) = send(
        &pool,
        "POST",
        &caps,
        &key,
        Some(json!({"name": "pushed", "consent": {"required": true}})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../migrations")]
async fn mcp_publish_takes_consent(pool: PgPool) {
    let f = seed_demo(&pool).await;
    let (alice_user, _) = alice(&pool, &f).await;
    let key = api_key(&pool, alice_user).await;
    let published = mcp(
        &pool,
        &key,
        "publish_capability",
        json!({
            "agent_id": f.granter_agent_id,
            "name": "book_room",
            "consent": {"required": true, "max_validity": "1d"},
        }),
    )
    .await;
    assert_eq!(published["consent"]["required"], true);
    assert_eq!(published["consent"]["max_validity"], "1d");
    assert_eq!(published["consent"]["wait"], "1d");
    let required: bool = sqlx::query_scalar(
        "SELECT consent_required FROM agent_capabilities WHERE name = 'book_room'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(required);
}
