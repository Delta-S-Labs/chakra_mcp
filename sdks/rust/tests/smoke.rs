//! Hermetic smoke - `httpmock` stands in for the backend so the suite
//! runs in CI without a server. Validates Bearer auth, error envelope
//! decoding, and the polling + serve helpers.

use std::future::IntoFuture;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chakramcp::{
    ChakraMCP, Error, HandlerResult, InvocationStatus, InvokeRequest, PollOpts,
};
use httpmock::prelude::*;
use serde_json::json;
use tokio_util::sync::CancellationToken;

mod _common {
    use super::*;
    pub fn build(server: &MockServer) -> ChakraMCP {
        ChakraMCP::builder()
            .api_key("ck_test")
            .app_url(server.base_url())
            .relay_url(server.base_url())
            .build()
            .expect("client builds")
    }
}

#[tokio::test]
async fn rejects_bad_api_key() {
    let err = ChakraMCP::new("not-a-key").err().expect("rejected");
    assert!(matches!(err, Error::InvalidApiKey));
}

#[tokio::test]
async fn me_sets_bearer_and_decodes() {
    let server = MockServer::start_async().await;
    let mock = server.mock_async(|when, then| {
        when.method(GET).path("/v1/me").header("authorization", "Bearer ck_test");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "user": {
                    "id": "u1",
                    "email": "alice@example.com",
                    "display_name": "Alice",
                    "avatar_url": null,
                    "is_admin": false
                },
                "memberships": [],
                "survey_required": false,
            }));
    }).await;
    let chakra = _common::build(&server);
    let me = chakra.me().await.expect("me");
    assert_eq!(me.user.email, "alice@example.com");
    mock.assert_async().await;
}

#[tokio::test]
async fn error_envelope_decoded() {
    let server = MockServer::start_async().await;
    server.mock_async(|when, then| {
        when.method(GET).path("/v1/agents");
        then.status(403)
            .header("content-type", "application/json")
            .json_body(json!({ "error": { "code": "forbidden", "message": "forbidden" } }));
    }).await;
    let chakra = _common::build(&server);
    let err = chakra.agents().list().await.unwrap_err();
    match err {
        Error::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "forbidden");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn invoke_and_wait_polls_until_terminal() {
    let server = MockServer::start_async().await;
    server.mock_async(|when, then| {
        when.method(POST).path("/v1/invoke");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "invocation_id": "inv1",
                "status": "pending",
                "error": null,
            }));
    }).await;
    server.mock_async(|when, then| {
        when.method(GET).path("/v1/invocations/inv1");
        then.status(200).header("content-type", "application/json").json_body(json!({
            "id": "inv1",
            "grant_id": "g1",
            "granter_agent_id": "a1",
            "granter_display_name": "Alice Bot",
            "grantee_agent_id": "a2",
            "grantee_display_name": "Bob Bot",
            "capability_id": "c1",
            "capability_name": "echo",
            "status": "succeeded",
            "elapsed_ms": 100,
            "error_message": null,
            "input_preview": {"hello": "world"},
            "output_preview": {"echoed": "world"},
            "created_at": "2026-01-01T00:00:00Z",
            "claimed_at": null,
            "i_served": false,
            "i_invoked": true,
        }));
    }).await;

    let chakra = _common::build(&server);
    let final_inv = chakra
        .invoke_and_wait(
            &InvokeRequest {
                grant_id: "g1".into(),
                grantee_agent_id: "a2".into(),
                input: json!({"hello": "world"}),
            },
            PollOpts {
                interval: Some(Duration::from_millis(5)),
                timeout: Some(Duration::from_secs(5)),
            },
        )
        .await
        .expect("succeeds");
    assert_eq!(final_inv.status, InvocationStatus::Succeeded);
    assert_eq!(
        final_inv.output_preview.as_ref().unwrap(),
        &json!({"echoed": "world"})
    );
}

#[tokio::test]
async fn inbox_serve_dispatches_and_responds_then_cancels() {
    let server = MockServer::start_async().await;
    let dispatched = Arc::new(AtomicUsize::new(0));

    server.mock_async(|when, then| {
        when.method(GET).path("/v1/inbox");
        then.status(200).json_body(json!([{
            "id": "inv1",
            "grant_id": null,
            "granter_agent_id": null,
            "granter_display_name": null,
            "grantee_agent_id": null,
            "grantee_display_name": null,
            "capability_id": null,
            "capability_name": "echo",
            "status": "in_progress",
            "elapsed_ms": 0,
            "error_message": null,
            "input_preview": {"hi": "there"},
            "output_preview": null,
            "created_at": "2026-01-01T00:00:00Z",
            "claimed_at": "2026-01-01T00:00:01Z",
            "i_served": true,
            "i_invoked": false
        }]));
    }).await;

    let result_mock = server.mock_async(|when, then| {
        when.method(POST).path("/v1/invocations/inv1/result");
        then.status(200).json_body(json!({}));
    }).await;

    let chakra = _common::build(&server);
    let cancel = CancellationToken::new();
    let cancel_for_handler = cancel.clone();
    let dispatched_for_handler = dispatched.clone();

    let serve_fut = chakra
        .inbox()
        .serve("agent-id", move |inv| {
            let cancel = cancel_for_handler.clone();
            let dispatched = dispatched_for_handler.clone();
            async move {
                assert_eq!(inv.id, "inv1");
                dispatched.fetch_add(1, Ordering::SeqCst);
                // Cancel after the first dispatch so the loop exits.
                cancel.cancel();
                Ok::<_, std::io::Error>(HandlerResult::Succeeded(json!({"ok": true})))
            }
        })
        .poll_interval(Duration::from_millis(20))
        .with_cancellation(cancel)
        .into_future();

    tokio::time::timeout(Duration::from_secs(5), serve_fut)
        .await
        .expect("serve does not hang")
        .expect("serve completes");

    assert!(dispatched.load(Ordering::SeqCst) >= 1);
    assert!(result_mock.hits_async().await >= 1);
}
