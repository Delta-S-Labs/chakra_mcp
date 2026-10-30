//! Claims and deadlines, hermetic: `httpmock` stands in for a 0.7 relay.
//! `release` hands a claim back, `expires_in` shortens a call's deadline,
//! and invocations carry `peer`, `expires_at`, `lease_until` and
//! `deliveries`.

use chakramcp::{ChakraMCP, Error, InvocationStatus, InvokeRequest};
use httpmock::prelude::*;
use serde_json::json;

fn build(server: &MockServer) -> ChakraMCP {
    ChakraMCP::builder()
        .api_key("ck_test")
        .app_url(server.base_url())
        .relay_url(server.base_url())
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn release_posts_and_returns_the_invocation() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/invocations/inv-1/release")
                .json_body(json!({}));
            then.status(200).json_body(json!({
                "id": "inv-1", "grant_id": null, "granter_agent_id": null,
                "granter_display_name": null, "grantee_agent_id": null,
                "grantee_display_name": null, "capability_id": null,
                "capability_name": "echo", "status": "pending", "elapsed_ms": 0,
                "error_message": null, "input_preview": {}, "output_preview": null,
                "created_at": "2026-01-01T00:00:00Z", "claimed_at": null,
                "i_served": true, "i_invoked": false,
                "expires_at": "2026-01-02T00:00:00Z", "lease_until": null, "deliveries": 0,
            }));
        })
        .await;
    let inv = build(&server)
        .inbox()
        .release("inv-1")
        .await
        .expect("releases");
    mock.assert_async().await;
    assert_eq!(inv.status, InvocationStatus::Pending);
    assert_eq!(inv.deliveries, 0);
    assert!(inv.lease_until.is_none());
    assert!(inv.expires_at.is_some());
}

#[tokio::test]
async fn a_refused_release_carries_the_code() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invocations/inv-1/release");
            then.status(409).json_body(json!({ "error": {
                "code": "chk.invocation.not_releasable",
                "message": "only a claimed invocation can be released" } }));
        })
        .await;
    match build(&server).inbox().release("inv-1").await {
        Err(Error::Api { status, code, .. }) => {
            assert_eq!(status, 409);
            assert_eq!(code, "chk.invocation.not_releasable");
        }
        other => panic!("expected a 409, got {other:?}"),
    }
}

#[tokio::test]
async fn invoke_sends_expires_in_and_reads_peer() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invoke").json_body(json!({
                "grant_id": "g-1", "grantee_agent_id": "a-1", "input": {}, "expires_in": 600 }));
            then.status(202).json_body(json!({
                "invocation_id": "inv-1", "status": "pending", "error": null,
                "peer": { "delivery": "pull", "online": false,
                          "last_seen_at": "2026-10-29T09:12:03Z" } }));
        })
        .await;
    let res = build(&server)
        .invoke(&InvokeRequest {
            grant_id: Some("g-1".into()),
            grantee_agent_id: "a-1".into(),
            input: json!({}),
            expires_in: Some(600),
            ..Default::default()
        })
        .await
        .expect("invokes");
    mock.assert_async().await;
    let peer = res.peer.expect("peer");
    assert_eq!(peer.delivery, "pull");
    assert!(!peer.online);
    assert!(peer.last_seen_at.is_some());
}

#[tokio::test]
async fn pulls_carry_the_owners_reply_link() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/inbox");
            then.status(200).json_body(json!([{
                "id": "inv-1", "grant_id": null, "granter_agent_id": null,
                "granter_display_name": null, "grantee_agent_id": null,
                "grantee_display_name": null, "capability_id": null,
                "capability_name": "message_owner", "status": "in_progress", "elapsed_ms": 0,
                "error_message": null, "input_preview": {}, "output_preview": null,
                "created_at": "2026-01-01T00:00:00Z", "claimed_at": null,
                "i_served": true, "i_invoked": false, "semantics": "human_in_loop",
                "owner_reply_url": "https://chakramcp.com/app/messages/inv-1",
            }]));
        })
        .await;
    let pulled = build(&server)
        .inbox()
        .pull("agent", None)
        .await
        .expect("pulls");
    assert_eq!(
        pulled[0].owner_reply_url.as_deref(),
        Some("https://chakramcp.com/app/messages/inv-1")
    );
}
