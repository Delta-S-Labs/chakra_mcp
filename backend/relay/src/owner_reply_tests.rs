//! The owner answers on the web (waking pull agents §9, W5a): an approver's
//! web session answers a `human_in_loop` call whether or not its agent
//! claimed it; everyone else follows the claim rules; approvers see the
//! calls waiting for them, and agents get the link to pass on.

use axum::http::StatusCode;
use chakramcp_shared::consent::decide::Actor;
use chakramcp_shared::consent::reply::{answer_as_owner, Draft};
use chakramcp_shared::error::ApiError;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::calls::tests::{calls_on, get, grant, mcp, person, post, world, Person, World};
use crate::state::RelayState;

async fn human_in_loop(pool: &PgPool, agent: Uuid) {
    sqlx::query("UPDATE agent_capabilities SET semantics = 'human_in_loop' WHERE agent_id = $1")
        .bind(agent)
        .execute(pool)
        .await
        .unwrap();
}

/// Alice's agent messages Bob's.
async fn message(state: &RelayState, w: &World, grant_id: Uuid) -> Uuid {
    let (status, body) = post(
        state,
        &w.alice.key,
        "/v1/invoke",
        json!({ "grant_id": grant_id, "grantee_agent_id": w.a, "input": { "message": "Lunch?" } }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body["invocation_id"].as_str().unwrap().parse().unwrap()
}

async fn answer(state: &RelayState, token: &str, id: Uuid) -> (StatusCode, Value) {
    post(
        state,
        token,
        &format!("/v1/invocations/{id}/result"),
        json!({ "status": "succeeded", "output": { "status": "replied", "reply": "Sure" } }),
    )
    .await
}

async fn row(pool: &PgPool, id: Uuid) -> (String, i32, Option<Uuid>) {
    sqlx::query_as(
        "SELECT status, deliveries, claimed_by_user_id FROM relay_invocations WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn notices(state: &RelayState, w: &World) -> Vec<String> {
    let (_, body) = get(
        state,
        &w.bob.key,
        &format!("/v1/inbox/notices?agent_id={}", w.b),
    )
    .await;
    body.as_array()
        .unwrap()
        .iter()
        .map(|n| n["kind"].as_str().unwrap().to_owned())
        .collect()
}

/// A member of Bob's account who isn't an approver (owners and admins are,
/// when no approvers are listed).
async fn member(pool: &PgPool, w: &World) -> Person {
    let carol = person(pool, "Carol").await;
    sqlx::query(
        "INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'member')",
    )
    .bind(Uuid::now_v7())
    .bind(w.bob.account)
    .bind(carol.user)
    .execute(pool)
    .await
    .unwrap();
    carol
}

#[sqlx::test(migrations = "../migrations")]
async fn an_approver_answers_a_call_nobody_claimed(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "message_owner").await;
    human_in_loop(&pool, w.b).await;
    let state = calls_on(&pool);
    let id = message(&state, &w, g).await;

    let (status, body) = answer(&state, &w.bob.web, id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "succeeded");
    assert_eq!(
        row(&pool, id).await,
        ("succeeded".into(), 1, Some(w.bob.user))
    );
    // Its agent never saw it, so it isn't told.
    assert!(notices(&state, &w).await.is_empty());
    // The caller sees a person stood behind it.
    let (_, seen) = get(&state, &w.alice.key, &format!("/v1/invocations/{id}")).await;
    assert_eq!(seen["output_preview"]["reply"], "Sure");
    assert!(
        seen["human_confirmed"]["consent_request_id"].is_string(),
        "{seen}"
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn a_claimed_call_tells_its_agent(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "message_owner").await;
    human_in_loop(&pool, w.b).await;
    let state = calls_on(&pool);
    let id = message(&state, &w, g).await;
    get(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;

    let (status, body) = answer(&state, &w.bob.web, id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&pool, id).await.0, "succeeded");
    assert_eq!(notices(&state, &w).await, ["reply_confirmed"]);
}

#[sqlx::test(migrations = "../migrations")]
async fn anyone_else_still_needs_the_claim(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "message_owner").await;
    human_in_loop(&pool, w.b).await;
    let state = calls_on(&pool);
    let carol = member(&pool, &w).await;
    let id = message(&state, &w, g).await;

    // An agent key, even the owner's, and a member who isn't an approver.
    for token in [&w.bob.key, &carol.web] {
        let (status, body) = answer(&state, token, id).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "chk.invocation.not_claimed");
    }
    assert_eq!(row(&pool, id).await.0, "pending");

    // Claimed, their reply is a draft for an approver.
    get(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    let (status, body) = answer(&state, &carol.web, id).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "awaiting_confirmation");
}

#[sqlx::test(migrations = "../migrations")]
async fn only_approvers_answer_and_only_open_human_in_loop_calls(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "book_venue").await;
    let state = calls_on(&pool);
    let carol = member(&pool, &w).await;
    let draft = || Draft {
        status: "succeeded".into(),
        output: Some(json!("ok")),
        error: None,
    };
    let as_user = |user| Actor {
        user_id: user,
        minted_jti: None,
    };
    let answer_as =
        |user, id| answer_as_owner(&pool, &state.wake, None, as_user(user), id, draft());

    // An autonomous capability is its agent's to answer.
    let id = message(&state, &w, g).await;
    assert!(matches!(
        answer_as(w.bob.user, id).await,
        Err(ApiError::Conflict(_))
    ));

    human_in_loop(&pool, w.b).await;
    let id = message(&state, &w, g).await;
    assert!(matches!(
        answer_as(carol.user, id).await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        answer_as(w.alice.user, id).await,
        Err(ApiError::Forbidden)
    ));
    let answered = answer_as(w.bob.user, id).await.unwrap();
    assert_eq!(answered.status, "succeeded");
    match answer_as(w.bob.user, id).await {
        Err(ApiError::Coded { code, .. }) => assert_eq!(code, "chk.invocation.already_settled"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        answer_as(w.bob.user, Uuid::now_v7()).await,
        Err(ApiError::NotFound)
    ));
}

#[sqlx::test(migrations = "../migrations")]
async fn approvers_see_the_calls_waiting_for_them(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "message_owner").await;
    human_in_loop(&pool, w.b).await;
    let state = calls_on(&pool);
    let carol = member(&pool, &w).await;
    let open = message(&state, &w, g).await;
    let drafted = message(&state, &w, g).await;
    // Its agent claims both and drafts a reply to the second.
    get(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    let (status, _) = answer(&state, &w.bob.key, drafted).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let list = |token: String| {
        let state = state.clone();
        async move {
            let (status, body) = get(&state, &token, "/v1/approvals?state=messages").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body
        }
    };
    let mine = list(w.bob.web.clone()).await;
    let items = mine.as_array().unwrap();
    assert_eq!(items.len(), 1, "the drafted one shows as its draft: {mine}");
    assert_eq!(items[0]["invocation_id"], open.to_string());
    assert_eq!(items[0]["status"], "in_progress");
    assert_eq!(
        items[0]["owner_reply_url"],
        format!("http://localhost:3000/app/messages/{open}")
    );
    assert_eq!(items[0]["capability_name"], "message_owner");
    // Not an approver, or not their agent: nothing.
    assert_eq!(list(carol.web.clone()).await, json!([]));
    assert_eq!(list(w.alice.web.clone()).await, json!([]));
    // An agent's key can't list approvals at all.
    let (status, _) = get(&state, &w.bob.key, "/v1/approvals?state=messages").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Answered: gone.
    let (status, _) = answer(&state, &w.bob.web, open).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list(w.bob.web.clone()).await, json!([]));
}

#[sqlx::test(migrations = "../migrations")]
async fn agents_get_the_link_to_pass_on(pool: PgPool) {
    let w = world(&pool).await;
    let ask = grant(&pool, w.b, w.a, "message_owner").await;
    let book = grant(&pool, w.b, w.a, "book_venue").await;
    sqlx::query(
        "UPDATE agent_capabilities SET semantics = 'human_in_loop' WHERE agent_id = $1 AND name = 'message_owner'",
    )
    .bind(w.b)
    .execute(&pool)
    .await
    .unwrap();
    let state = calls_on(&pool);
    let asked = message(&state, &w, ask).await;
    let booked = message(&state, &w, book).await;
    let link = format!("http://localhost:3000/app/messages/{asked}");

    let (_, pulled) = get(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    let item = |id: Uuid| {
        pulled
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id.to_string())
            .unwrap()
            .clone()
    };
    assert_eq!(item(asked)["owner_reply_url"], link);
    assert!(
        item(booked).get("owner_reply_url").is_none(),
        "only human_in_loop"
    );

    // The served side's reads carry it; the caller's don't.
    let (_, served) = get(&state, &w.bob.key, &format!("/v1/invocations/{asked}")).await;
    assert_eq!(served["owner_reply_url"], link);
    let (_, called) = get(&state, &w.alice.key, &format!("/v1/invocations/{asked}")).await;
    assert!(called.get("owner_reply_url").is_none(), "{called}");

    // MCP too.
    let polled = mcp(
        &state,
        &w.bob.key,
        "tools/call",
        json!({ "name": "poll_invocation", "arguments": { "invocation_id": asked } }),
    )
    .await;
    assert_eq!(polled["structuredContent"]["owner_reply_url"], link);
}

#[sqlx::test(migrations = "../migrations")]
async fn approvers_read_the_call_they_answer(pool: PgPool) {
    let w = world(&pool).await;
    let g = grant(&pool, w.b, w.a, "message_owner").await;
    sqlx::query(
        "UPDATE agent_capabilities SET semantics = 'human_in_loop', output_schema = $2
          WHERE agent_id = $1",
    )
    .bind(w.b)
    .bind(json!({ "type": "object", "properties": {
        "status": { "type": "string", "enum": ["replied"] }, "reply": { "type": "string" } } }))
    .execute(&pool)
    .await
    .unwrap();
    let state = calls_on(&pool);
    let carol = member(&pool, &w).await;
    let id = message(&state, &w, g).await;
    let path = format!("/v1/approvals/messages/{id}");

    let (status, view) = get(&state, &w.bob.web, &path).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["status"], "pending");
    assert_eq!(view["reply_shape"], "replied");
    assert_eq!(view["answerable"], true);
    assert_eq!(view["input"]["message"], "Lunch?");
    assert_eq!(view["capability_name"], "message_owner");
    assert!(view.get("draft_approve_url").is_none());
    // Not an approver: no such message. An agent key: not a person.
    assert_eq!(
        get(&state, &carol.web, &path).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&state, &w.bob.key, &path).await.0,
        StatusCode::FORBIDDEN
    );

    // Its agent drafts a reply: confirmed on the approval page instead.
    get(&state, &w.bob.key, &format!("/v1/inbox?agent_id={}", w.b)).await;
    answer(&state, &w.bob.key, id).await;
    let (_, view) = get(&state, &w.bob.web, &path).await;
    assert_eq!(view["answerable"], false);
    assert!(view["draft_approve_url"]
        .as_str()
        .unwrap()
        .starts_with("http://localhost:3000/app/approvals/"));
}
