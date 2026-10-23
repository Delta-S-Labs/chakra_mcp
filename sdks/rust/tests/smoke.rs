//! Hermetic smoke - `httpmock` stands in for the backend so the suite
//! runs in CI without a server. Validates Bearer auth, error envelope
//! decoding, and the polling + serve helpers.

use std::future::IntoFuture;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chakramcp::{
    ChakraMCP, Error, HandlerResult, InvocationStatus, InvokeRequest, PollOpts, RespondOutcome,
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
    let mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/me")
                .header("authorization", "Bearer ck_test");
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
        })
        .await;
    let chakra = _common::build(&server);
    let me = chakra.me().await.expect("me");
    assert_eq!(me.user.email, "alice@example.com");
    mock.assert_async().await;
}

#[tokio::test]
async fn error_envelope_decoded() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/agents");
            then.status(403)
                .header("content-type", "application/json")
                .json_body(json!({ "error": { "code": "forbidden", "message": "forbidden" } }));
        })
        .await;
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
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invoke");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "invocation_id": "inv1",
                    "status": "pending",
                    "error": null,
                }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/invocations/inv1");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
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
        })
        .await;

    let chakra = _common::build(&server);
    let final_inv = chakra
        .invoke_and_wait(
            &InvokeRequest {
                grant_id: Some("g1".into()),
                capability_id: None,
                grantee_agent_id: "a2".into(),
                input: json!({"hello": "world"}),
                ..Default::default()
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
async fn respond_tells_a_draft_from_a_finished_call_and_notices_decode() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invocations/hitl/result");
            then.status(202).json_body(json!({
                "status": "awaiting_confirmation",
                "consent_request_id": "cr-1",
                "approve_url": "https://chakramcp.com/app/approvals/cr-1",
                "expires_at": "2026-10-24T00:00:00Z",
            }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invocations/auto/result");
            then.status(200).json_body(json!({
                "id": "auto", "grant_id": null, "granter_agent_id": null,
                "granter_display_name": null, "grantee_agent_id": null,
                "grantee_display_name": null, "capability_id": null,
                "capability_name": "echo", "status": "succeeded", "elapsed_ms": 4,
                "error_message": null, "input_preview": {}, "output_preview": {"ok": true},
                "created_at": "2026-01-01T00:00:00Z", "claimed_at": null,
                "i_served": true, "i_invoked": false,
            }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/notices")
                .query_param("agent_id", "a1")
                .query_param("limit", "5");
            then.status(200).json_body(json!([{
                "id": "n1",
                "kind": "consent_requested",
                "created_at": "2026-10-23T00:00:00Z",
                "consent_request": {
                    "id": "cr-9", "invocation_id": "inv9", "status": "pending",
                    "capability_id": "cap", "capability_name": "book_meeting",
                    "requester": {
                        "agent_id": "a2", "agent_display_name": "Scheduler",
                        "account_id": "acc", "account_display_name": null, "verified": true,
                    },
                    "terms": {"valid_for": "once"},
                    "expires_at": "2026-10-24T00:00:00Z",
                    "approve_url": "https://chakramcp.com/app/approvals/cr-9",
                },
            }]));
        })
        .await;

    let chakra = _common::build(&server);
    let inbox = chakra.inbox();
    match inbox
        .respond_succeeded("hitl", json!({"reply": "hi"}))
        .await
        .unwrap()
    {
        RespondOutcome::Drafted(d) => assert_eq!(d.consent_request_id, "cr-1"),
        other => panic!("expected a draft, got {other:?}"),
    }
    match inbox
        .respond_succeeded("auto", json!({"ok": true}))
        .await
        .unwrap()
    {
        RespondOutcome::Finished(inv) => assert_eq!(inv.status, InvocationStatus::Succeeded),
        other => panic!("expected a finished call, got {other:?}"),
    }
    let notices = inbox.notices("a1", Some(5)).await.unwrap();
    let req = notices[0].consent_request.as_ref().unwrap();
    assert_eq!(notices[0].kind, "consent_requested");
    assert_eq!(req.capability_name.as_deref(), Some("book_meeting"));
    assert_eq!(req.terms.valid_for.as_deref(), Some("once"));
}

#[tokio::test]
async fn inbox_serve_dispatches_and_responds_then_cancels() {
    let server = MockServer::start_async().await;
    let dispatched = Arc::new(AtomicUsize::new(0));

    server
        .mock_async(|when, then| {
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
        })
        .await;

    let result_mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invocations/inv1/result");
            then.status(200).json_body(json!({}));
        })
        .await;

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
    assert!(result_mock.calls_async().await >= 1);
}

// ─── Usage limits ─────────────────────────────────────────

/// Run `invoke` against a server that answers with one canned error
/// and return what it got back.
async fn invoke_error(status: u16, retry_after: Option<&str>, body: serde_json::Value) -> Error {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invoke");
            let mut then = then
                .status(status)
                .header("content-type", "application/json");
            if let Some(v) = retry_after {
                then = then.header("retry-after", v);
            }
            then.json_body(body);
        })
        .await;
    let chakra = _common::build(&server);
    chakra
        .invoke(&InvokeRequest {
            grant_id: None,
            capability_id: Some("cap".into()),
            grantee_agent_id: "agent".into(),
            input: json!({}),
            ..Default::default()
        })
        .await
        .expect_err("invoke fails")
}

fn next_grant() -> chrono::DateTime<chrono::Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}

#[tokio::test]
async fn rate_limited_uses_retry_after_header() {
    let err = invoke_error(
        429,
        Some("23"),
        json!({ "error": {
            "code": "account_rate_limited",
            "message": "rate limit exceeded",
            "retryable": true,
            "retry_after_secs": 30,
        }}),
    )
    .await;
    // The header wins over the body.
    assert_eq!(err.retry_after_secs(), Some(23));
    match err {
        Error::RateLimited {
            message,
            retry_after_secs,
        } => {
            assert_eq!(message, "rate limit exceeded");
            assert_eq!(retry_after_secs, Some(23));
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn credits_exhausted_with_credits_url() {
    let err = invoke_error(
        429,
        Some("1300000"),
        json!({ "error": {
            "code": "account_credits_exhausted",
            "message": "insufficient credits",
            "retryable": false,
            "retry_after_secs": 1300000,
            "next_grant_at": "2026-11-01T00:00:00Z",
            "credits_url": "https://chakramcp.com/app/credits",
        }}),
    )
    .await;
    match err {
        Error::CreditsExhausted {
            retry_after_secs,
            next_grant_at,
            credits_url,
            ..
        } => {
            assert_eq!(retry_after_secs, Some(1_300_000));
            assert_eq!(next_grant_at, Some(next_grant()));
            assert_eq!(
                credits_url.as_deref(),
                Some("https://chakramcp.com/app/credits")
            );
        }
        other => panic!("expected CreditsExhausted, got {other:?}"),
    }
}

#[tokio::test]
async fn credits_exhausted_self_hosted_has_no_credits_url() {
    let err = invoke_error(
        429,
        Some("86400"),
        json!({ "error": {
            "code": "account_credits_exhausted",
            "message": "insufficient credits",
            "retryable": false,
            "retry_after_secs": 86400,
            "next_grant_at": "2026-11-01T00:00:00Z",
        }}),
    )
    .await;
    match err {
        Error::CreditsExhausted {
            retry_after_secs,
            next_grant_at,
            credits_url,
            ..
        } => {
            assert_eq!(retry_after_secs, Some(86400));
            assert_eq!(next_grant_at, Some(next_grant()));
            assert_eq!(credits_url, None);
        }
        other => panic!("expected CreditsExhausted, got {other:?}"),
    }
}

#[tokio::test]
async fn retry_after_from_body_without_header() {
    let err = invoke_error(
        429,
        None,
        json!({ "error": {
            "code": "account_rate_limited",
            "message": "rate limit exceeded",
            "retryable": true,
            "retry_after_secs": 17,
        }}),
    )
    .await;
    assert!(
        matches!(
            err,
            Error::RateLimited {
                retry_after_secs: Some(17),
                ..
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn quota_exhausted_gains_retry_after() {
    let err = invoke_error(
        429,
        Some("1300000"),
        json!({
            "error": {
                "code": "monthly_quota_exhausted",
                "message": "monthly quota exhausted",
                "retryable": false,
            },
            "quota": 100,
            "resets_at": "2026-11-01T00:00:00Z",
        }),
    )
    .await;
    match err {
        Error::QuotaExhausted {
            quota,
            resets_at,
            retry_after_secs,
            ..
        } => {
            assert_eq!(quota, 100);
            assert_eq!(resets_at, next_grant());
            assert_eq!(retry_after_secs, Some(1_300_000));
        }
        other => panic!("expected QuotaExhausted, got {other:?}"),
    }
}

#[tokio::test]
async fn older_server_limit_errors_have_no_new_fields() {
    let credits = invoke_error(
        429,
        None,
        json!({ "error": {
            "code": "account_credits_exhausted",
            "message": "insufficient credits",
            "retryable": false,
        }}),
    )
    .await;
    assert!(
        matches!(
            credits,
            Error::CreditsExhausted {
                retry_after_secs: None,
                next_grant_at: None,
                credits_url: None,
                ..
            }
        ),
        "got {credits:?}"
    );

    let rate = invoke_error(
        429,
        None,
        json!({ "error": { "code": "account_rate_limited", "message": "rate limit exceeded" } }),
    )
    .await;
    assert!(
        matches!(
            rate,
            Error::RateLimited {
                retry_after_secs: None,
                ..
            }
        ),
        "got {rate:?}"
    );

    let quota = invoke_error(
        429,
        None,
        json!({
            "error": { "code": "monthly_quota_exhausted", "message": "monthly quota exhausted" },
            "quota": 100,
            "resets_at": "2026-11-01T00:00:00Z",
        }),
    )
    .await;
    assert!(
        matches!(
            quota,
            Error::QuotaExhausted {
                retry_after_secs: None,
                ..
            }
        ),
        "got {quota:?}"
    );
}

#[tokio::test]
async fn other_errors_carry_retry_after_only_when_sent() {
    let forbidden = invoke_error(
        403,
        None,
        json!({ "error": { "code": "forbidden", "message": "forbidden" } }),
    )
    .await;
    assert!(
        matches!(
            forbidden,
            Error::Api {
                status: 403,
                retry_after_secs: None,
                ..
            }
        ),
        "got {forbidden:?}"
    );

    let unavailable = invoke_error(
        503,
        Some("5"),
        json!({ "error": { "code": "unavailable", "message": "try later" } }),
    )
    .await;
    assert!(
        matches!(
            unavailable,
            Error::Api {
                status: 503,
                retry_after_secs: Some(5),
                ..
            }
        ),
        "got {unavailable:?}"
    );
}
