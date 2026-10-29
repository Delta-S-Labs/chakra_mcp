//! Waiting pulls, hermetic: `httpmock` stands in for the relay. A relay
//! from 0.7 holds a pull up to `wait` seconds and says how long in the
//! `ChakraMCP-Wait` header; an older one answers at once without it.

use std::future::IntoFuture;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chakramcp::{ChakraMCP, Error, HandlerResult, InboxPullQuery, InviteAnswer, PullQuery};
use httpmock::prelude::*;
use httpmock::Mock;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const WAIT_HEADER: &str = "ChakraMCP-Wait";

fn build(server: &MockServer) -> ChakraMCP {
    ChakraMCP::builder()
        .api_key("ck_test")
        .app_url(server.base_url())
        .relay_url(server.base_url())
        .build()
        .expect("client builds")
}

fn invocation_json(id: &str) -> Value {
    json!({
        "id": id, "grant_id": null, "granter_agent_id": null,
        "granter_display_name": null, "grantee_agent_id": null,
        "grantee_display_name": null, "capability_id": null,
        "capability_name": "echo", "status": "in_progress", "elapsed_ms": 0,
        "error_message": null, "input_preview": {"hi": "there"}, "output_preview": null,
        "created_at": "2026-01-01T00:00:00Z", "claimed_at": "2026-01-01T00:00:01Z",
        "i_served": true, "i_invoked": false,
    })
}

fn invite_json(call_id: &str) -> Value {
    json!({
        "call": {
            "id": call_id, "state": "ringing",
            "caller": {
                "agent_id": "caller-agent", "account_id": "acc-1",
                "display_name": "Scheduler", "account_slug": "acme",
            },
            "callee": {
                "agent_id": "me", "account_id": "acc-2",
                "display_name": "Helper", "account_slug": "mine",
            },
            "memory": "own",
            "memory_features": {"tags": false, "recall": false},
            "ttl_secs": 2592000, "idle_secs": 86400, "topic": null,
            "created_at": "2026-10-26T10:00:00Z", "answered_at": null,
            "ended_at": null, "ended_by": null, "purge_at": null, "purged_at": null,
            "message_count": 0, "last_message_at": null,
        },
        "entry": {
            "seq": 1, "kind": "invite", "from_agent_id": "caller-agent",
            "text": "Hi", "data": null, "blocked": false,
            "created_at": "2026-10-26T10:01:00Z",
        },
    })
}

/// Wait until `mock` has been hit `n` times, then fire `cancel`.
async fn cancel_after_hits(mock: &Mock<'_>, n: usize, cancel: &CancellationToken) {
    while mock.calls_async().await < n {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cancel.cancel();
}

// ─── pull, notices and calls pull ─────────────────────────

#[tokio::test]
async fn pull_sends_wait_and_lease_secs_only_when_set() {
    let server = MockServer::start_async().await;
    let waiting = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param("agent_id", "a1")
                .query_param("limit", "5")
                .query_param("wait", "25")
                .query_param("lease_secs", "1800");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .json_body(json!([invocation_json("inv1")]));
        })
        .await;
    let plain = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param("agent_id", "a1")
                .query_param("limit", "3")
                .query_param_missing("wait")
                .query_param_missing("lease_secs");
            then.status(200).json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let inbox = chakra.inbox();

    let got = inbox
        .pull(
            "a1",
            InboxPullQuery {
                limit: Some(5),
                wait: Some(25),
                lease_secs: Some(1800),
            },
        )
        .await
        .expect("waiting pull");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, "inv1");
    waiting.assert_async().await;

    // A bare limit, as before: no wait, no lease.
    assert!(inbox.pull("a1", Some(3)).await.expect("pull").is_empty());
    plain.assert_async().await;
}

#[tokio::test]
async fn notices_take_wait_and_carry_invocation_id() {
    let server = MockServer::start_async().await;
    let notices = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/notices")
                .query_param("agent_id", "a1")
                .query_param("wait", "10")
                .query_param_missing("limit");
            then.status(200).header(WAIT_HEADER, "10").json_body(json!([
                {"id": "n1", "kind": "invocation_expired",
                 "created_at": "2026-10-29T00:00:00Z", "invocation_id": "inv-7"},
                {"id": "n2", "kind": "wake_url_disabled",
                 "created_at": "2026-10-29T00:00:01Z"},
            ]));
        })
        .await;
    let chakra = build(&server);

    let got = chakra
        .inbox()
        .notices(
            "a1",
            PullQuery {
                wait: Some(10),
                ..Default::default()
            },
        )
        .await
        .expect("notices");
    notices.assert_async().await;
    assert_eq!(got[0].invocation_id.as_deref(), Some("inv-7"));
    assert_eq!(got[1].invocation_id, None);
}

#[tokio::test]
async fn calls_pull_takes_wait() {
    let server = MockServer::start_async().await;
    let waiting = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "me")
                .query_param("limit", "10")
                .query_param("wait", "20");
            then.status(200)
                .header(WAIT_HEADER, "20")
                .json_body(json!([invite_json("c1")]));
        })
        .await;
    let plain = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "me")
                .query_param_missing("wait")
                .query_param_missing("limit");
            then.status(200).json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let calls = chakra.calls();

    let got = calls
        .pull(
            "me",
            PullQuery {
                limit: Some(10),
                wait: Some(20),
            },
        )
        .await
        .expect("waiting pull");
    assert_eq!(got[0].call.id, "c1");
    waiting.assert_async().await;

    assert!(calls.pull("me", None).await.expect("pull").is_empty());
    plain.assert_async().await;
}

// ─── pull_with_meta ───────────────────────────────────────

#[tokio::test]
async fn pull_with_meta_reads_the_wait_header() {
    let server = MockServer::start_async().await;
    for (agent, header) in [
        ("waited", Some("25")),
        ("busy", Some("0")),
        ("garbled", Some("soon")),
        ("older", None),
    ] {
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/inbox")
                    .query_param("agent_id", agent);
                let then = then.status(200);
                let then = match header {
                    Some(v) => then.header(WAIT_HEADER, v),
                    None => then,
                };
                then.json_body(json!([]));
            })
            .await;
    }
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "waited");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .json_body(json!([invite_json("c1")]));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "older");
            then.status(200).json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let wait = InboxPullQuery {
        wait: Some(25),
        ..Default::default()
    };

    let inbox = chakra.inbox();
    let waited = |agent: &'static str| {
        let inbox = &inbox;
        async move {
            inbox
                .pull_with_meta(agent, wait)
                .await
                .expect("pull")
                .waited
        }
    };
    assert_eq!(waited("waited").await, Some(25));
    assert_eq!(waited("busy").await, Some(0));
    assert_eq!(waited("garbled").await, None);
    assert_eq!(waited("older").await, None);

    let calls = chakra.calls();
    let got = calls
        .pull_with_meta(
            "waited",
            PullQuery {
                wait: Some(25),
                ..Default::default()
            },
        )
        .await
        .expect("calls pull");
    assert_eq!(got.waited, Some(25));
    assert_eq!(got.items[0].call.id, "c1");
    let got = calls
        .pull_with_meta("older", None)
        .await
        .expect("calls pull");
    assert_eq!(got.waited, None);
    assert!(got.items.is_empty());
}

#[tokio::test]
async fn pull_with_meta_maps_errors_as_before() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param("agent_id", "limited");
            then.status(429)
                .header("Retry-After", "7")
                .header(WAIT_HEADER, "0")
                .json_body(json!({
                    "error": {"code": "account_rate_limited", "message": "slow down"},
                }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "theirs");
            then.status(403)
                .json_body(json!({"error": {"code": "forbidden", "message": "not yours"}}));
        })
        .await;
    let chakra = build(&server);

    let err = chakra
        .inbox()
        .pull_with_meta("limited", InboxPullQuery::default())
        .await
        .expect_err("rate limited");
    match err {
        Error::RateLimited {
            message,
            retry_after_secs,
        } => {
            assert_eq!(message, "slow down");
            assert_eq!(retry_after_secs, Some(7));
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
    let err = chakra
        .calls()
        .pull_with_meta("theirs", None)
        .await
        .expect_err("forbidden");
    assert!(
        matches!(&err, Error::Api { status: 403, code, message, .. }
            if code == "forbidden" && message == "not yours"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn a_waiting_pull_outlasts_a_short_client_timeout() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/inbox").query_param("wait", "1");
            then.status(200)
                .header(WAIT_HEADER, "1")
                .delay(Duration::from_millis(800))
                .json_body(json!([]));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param_missing("wait");
            then.status(200)
                .delay(Duration::from_millis(800))
                .json_body(json!([]));
        })
        .await;
    let chakra = ChakraMCP::builder()
        .api_key("ck_test")
        .relay_url(server.base_url())
        .request_timeout(Duration::from_millis(300))
        .build()
        .expect("client builds");
    let inbox = chakra.inbox();

    // The pull may take its wait and 30 s more, whatever the client's
    // timeout.
    let got = inbox
        .pull_with_meta(
            "a1",
            InboxPullQuery {
                wait: Some(1),
                ..Default::default()
            },
        )
        .await
        .expect("a waiting pull isn't cut off by the client's timeout");
    assert_eq!(got.waited, Some(1));

    // Without wait, the client's timeout holds.
    let err = inbox.pull("a1", None).await.expect_err("times out");
    assert!(
        matches!(&err, Error::Transport(e) if e.is_timeout()),
        "got {err:?}"
    );
}

// ─── inbox.serve ──────────────────────────────────────────

#[tokio::test]
async fn inbox_serve_pulls_again_at_once_while_the_relay_waits() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param("agent_id", "agent-1")
                .query_param("wait", "25");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = chakra
        .inbox()
        .serve("agent-1", |_inv| async {
            Ok::<_, std::io::Error>(HandlerResult::Succeeded(json!({})))
        })
        // Far longer than the test: only pulling again at once gets
        // several pulls in.
        .poll_interval(Duration::from_secs(30))
        .with_cancellation(cancel.clone());

    let (served, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(serve.into_future(), cancel_after_hits(&feed, 5, &cancel))
    })
    .await
    .expect("serve pulls again without sleeping");
    served.expect("serve completes");
    assert!(feed.calls_async().await >= 5);
}

#[tokio::test]
async fn inbox_serve_sleeps_when_the_relay_did_not_wait() {
    // No header (a relay before 0.7), and `0` (one that didn't wait).
    for header in [None, Some("0")] {
        let server = MockServer::start_async().await;
        let feed = server
            .mock_async(|when, then| {
                when.method(GET).path("/v1/inbox");
                let then = then.status(200);
                let then = match header {
                    Some(v) => then.header(WAIT_HEADER, v),
                    None => then,
                };
                then.json_body(json!([]));
            })
            .await;
        let chakra = build(&server);
        let cancel = CancellationToken::new();
        let serve = chakra
            .inbox()
            .serve("agent-1", |_inv| async {
                Ok::<_, std::io::Error>(HandlerResult::Succeeded(json!({})))
            })
            .poll_interval(Duration::from_secs(30))
            .with_cancellation(cancel.clone());
        let canceller = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel.cancel();
        };
        let (served, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(serve.into_future(), canceller)
        })
        .await
        .expect("serve stops when cancelled");
        served.expect("serve completes");
        assert_eq!(feed.calls_async().await, 1, "header {header:?}");
    }
}

#[tokio::test]
async fn inbox_serve_sends_wait_25_by_default() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param("agent_id", "agent-1")
                .query_param("limit", "25")
                .query_param("wait", "25")
                .query_param_missing("lease_secs");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .json_body(json!([invocation_json("inv1")]));
        })
        .await;
    let result = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invocations/inv1/result");
            then.status(200).json_body(invocation_json("inv1"));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let dispatched = Arc::new(AtomicUsize::new(0));
    let serve = {
        let stop = cancel.clone();
        let dispatched = dispatched.clone();
        chakra
            .inbox()
            .serve("agent-1", move |inv| {
                let cancel = stop.clone();
                let dispatched = dispatched.clone();
                async move {
                    assert_eq!(inv.id, "inv1");
                    dispatched.fetch_add(1, Ordering::SeqCst);
                    cancel.cancel();
                    Ok::<_, std::io::Error>(HandlerResult::Succeeded(json!({"ok": true})))
                }
            })
            // A pull that matched no mock would fail and sleep this long.
            .poll_interval(Duration::from_secs(30))
            .with_cancellation(cancel.clone())
            .into_future()
    };
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .expect("serve sent wait=25")
        .expect("serve completes");
    assert_eq!(feed.calls_async().await, 1);
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert_eq!(result.calls_async().await, 1);
}

#[tokio::test]
async fn inbox_serve_passes_lease_secs_and_wait_secs_zero_sends_no_wait() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox")
                .query_param("agent_id", "agent-1")
                .query_param("lease_secs", "1800")
                .query_param_missing("wait");
            then.status(200).json_body(json!([invocation_json("inv1")]));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invocations/inv1/result");
            then.status(200).json_body(invocation_json("inv1"));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = {
        let stop = cancel.clone();
        chakra
            .inbox()
            .serve("agent-1", move |_inv| {
                let cancel = stop.clone();
                async move {
                    cancel.cancel();
                    Ok::<_, std::io::Error>(HandlerResult::Succeeded(json!({})))
                }
            })
            .wait_secs(0)
            .lease_secs(1800)
            .poll_interval(Duration::from_secs(30))
            .with_cancellation(cancel.clone())
            .into_future()
    };
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .expect("serve sent lease_secs and no wait")
        .expect("serve completes");
    assert_eq!(feed.calls_async().await, 1);
}

#[tokio::test]
async fn inbox_serve_stops_mid_wait_when_cancelled() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/inbox");
            // A relay holding the pull for the whole wait.
            then.status(200)
                .header(WAIT_HEADER, "25")
                .delay(Duration::from_secs(25))
                .json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = chakra
        .inbox()
        .serve("agent-1", |_inv| async {
            Ok::<_, std::io::Error>(HandlerResult::Succeeded(json!({})))
        })
        .with_cancellation(cancel.clone());

    let (served, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(serve.into_future(), cancel_after_hits(&feed, 1, &cancel))
    })
    .await
    .expect("serve stops mid-wait");
    served.expect("serve completes");
}

// ─── calls.serve ──────────────────────────────────────────

#[tokio::test]
async fn calls_serve_pulls_again_at_once_while_the_relay_waits() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "me")
                .query_param("wait", "25");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = chakra
        .calls()
        .serve("me")
        .poll_interval(Duration::from_secs(30))
        .with_cancellation(cancel.clone());

    let (served, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(serve.into_future(), cancel_after_hits(&feed, 5, &cancel))
    })
    .await
    .expect("serve pulls again without sleeping");
    served.expect("serve completes");
    assert!(feed.calls_async().await >= 5);
}

#[tokio::test]
async fn calls_serve_sleeps_when_the_relay_did_not_wait() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/inbox/calls");
            then.status(200).json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = chakra
        .calls()
        .serve("me")
        .poll_interval(Duration::from_secs(30))
        .with_cancellation(cancel.clone());
    let canceller = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    };
    let (served, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(serve.into_future(), canceller)
    })
    .await
    .expect("serve stops when cancelled");
    served.expect("serve completes");
    assert_eq!(feed.calls_async().await, 1);
}

#[tokio::test]
async fn calls_serve_sends_wait_25_by_default() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "me")
                .query_param("limit", "25")
                .query_param("wait", "25");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .json_body(json!([invite_json("c1")]));
        })
        .await;
    let accept = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/calls/c1/accept");
            then.status(200)
                .json_body(invite_json("c1")["call"].clone());
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = {
        let stop = cancel.clone();
        chakra
            .calls()
            .serve("me")
            .on_invite(move |_item| {
                let cancel = stop.clone();
                async move {
                    cancel.cancel();
                    Ok::<_, std::convert::Infallible>(InviteAnswer::accept())
                }
            })
            // A pull that matched no mock would fail and sleep this long.
            .poll_interval(Duration::from_secs(30))
            .on_error(|e| panic!("unexpected serve error: {e}"))
            .with_cancellation(cancel.clone())
    };
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .expect("serve sent wait=25")
        .expect("serve completes");
    assert_eq!(feed.calls_async().await, 1);
    assert_eq!(accept.calls_async().await, 1);
}

#[tokio::test]
async fn calls_serve_stops_mid_wait_when_cancelled() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/inbox/calls");
            then.status(200)
                .header(WAIT_HEADER, "25")
                .delay(Duration::from_secs(25))
                .json_body(json!([]));
        })
        .await;
    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = chakra.calls().serve("me").with_cancellation(cancel.clone());

    let (served, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(serve.into_future(), cancel_after_hits(&feed, 1, &cancel))
    })
    .await
    .expect("serve stops mid-wait");
    served.expect("serve completes");
}
