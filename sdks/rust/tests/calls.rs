//! Calls, hermetic: `httpmock` stands in for the relay. Covers the
//! `calls()` routes, the `serve` loop, and `call_id` on invoke.

use std::future::IntoFuture;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chakramcp::{
    AcceptCalls, CallEntryKind, CallHistoryQuery, CallMemoryMode, CallReply, CallState,
    CallsServeError, ChakraMCP, Error, InviteAnswer, InvocationStatus, InvokeRequest,
    ListCallsQuery, StartCallRequest, UpdateAgentRequest,
};
use httpmock::prelude::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn build(server: &MockServer) -> ChakraMCP {
    ChakraMCP::builder()
        .api_key("ck_test")
        .app_url(server.base_url())
        .relay_url(server.base_url())
        .build()
        .expect("client builds")
}

fn call_json(id: &str, state: &str) -> Value {
    json!({
        "id": id,
        "state": state,
        "caller": {
            "agent_id": "caller-agent", "account_id": "acc-1",
            "display_name": "Scheduler", "account_slug": "acme",
        },
        "callee": {
            "agent_id": "me", "account_id": "acc-2",
            "display_name": "Helper", "account_slug": "mine",
        },
        "memory": "call",
        "memory_features": {"tags": true, "recall": true},
        "ttl_secs": 2592000,
        "idle_secs": 86400,
        "topic": "Book a meeting",
        "created_at": "2026-10-26T10:00:00Z",
        "answered_at": null,
        "ended_at": null,
        "ended_by": null,
        "purge_at": null,
        "purged_at": null,
        "message_count": 0,
        "last_message_at": null,
    })
}

fn entry_json(seq: i64, kind: &str, text: Option<&str>) -> Value {
    json!({
        "seq": seq,
        "kind": kind,
        "from_agent_id": "caller-agent",
        "text": text,
        "data": null,
        "blocked": false,
        "created_at": "2026-10-26T10:01:00Z",
    })
}

fn item_json(call_id: &str, seq: i64, kind: &str, text: Option<&str>) -> Value {
    json!({ "call": call_json(call_id, "active"), "entry": entry_json(seq, kind, text) })
}

#[tokio::test]
async fn start_sends_the_body_and_parses_a_call() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/calls").json_body(json!({
                "from_agent_id": "caller-agent",
                "to_agent_id": "me",
                "memory": "call",
                "ttl_secs": 0,
                "topic": "Book a meeting",
                "message": "Hi, can we find a slot?",
            }));
            then.status(201).json_body(call_json("c1", "ringing"));
        })
        .await;

    let call = build(&server)
        .calls()
        .start(&StartCallRequest {
            from_agent_id: "caller-agent".into(),
            to_agent_id: "me".into(),
            memory: Some(CallMemoryMode::Call),
            ttl_secs: Some(0),
            topic: Some("Book a meeting".into()),
            message: Some("Hi, can we find a slot?".into()),
            ..Default::default()
        })
        .await
        .expect("starts");
    mock.assert_async().await;
    assert_eq!(call.id, "c1");
    assert_eq!(call.state, CallState::Ringing);
    assert!(!call.state.is_final());
    assert_eq!(call.memory, CallMemoryMode::Call);
    assert!(call.memory_features.tags && call.memory_features.recall);
    assert_eq!((call.ttl_secs, call.idle_secs), (2_592_000, 86_400));
    assert_eq!(call.topic.as_deref(), Some("Book a meeting"));
    assert_eq!(call.caller.agent_id.as_deref(), Some("caller-agent"));
    assert_eq!(call.callee.account_slug.as_deref(), Some("mine"));
    assert_eq!(call.answered_at, None);
    assert_eq!(call.ended_by, None);
}

#[tokio::test]
async fn list_passes_the_query_and_reads_next_cursor() {
    let server = MockServer::start_async().await;
    let mut ended = call_json("c2", "ended");
    ended["ended_by"] = json!("idle");
    let mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/calls")
                .query_param("agent_id", "me")
                .query_param("state", "active")
                .query_param("limit", "10")
                .query_param("cursor", "abc");
            then.status(200).json_body(json!({
                "calls": [call_json("c1", "active"), ended],
                "next_cursor": "next-page",
            }));
        })
        .await;

    let page = build(&server)
        .calls()
        .list(ListCallsQuery {
            agent_id: Some("me".into()),
            state: Some(CallState::Active),
            limit: Some(10),
            cursor: Some("abc".into()),
        })
        .await
        .expect("lists");
    mock.assert_async().await;
    assert_eq!(page.next_cursor.as_deref(), Some("next-page"));
    assert_eq!(page.calls.len(), 2);
    assert_eq!(page.calls[0].state, CallState::Active);
    assert!(page.calls[1].state.is_final());
    assert_eq!(page.calls[1].ended_by, Some(chakramcp::CallEndedBy::Idle));
}

#[tokio::test]
async fn send_and_history_round_trip() {
    let server = MockServer::start_async().await;
    let send = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/calls/c1/messages")
                .json_body(json!({
                    "agent_id": "me",
                    "text": "Wednesday at 3 works",
                    "data": {"slot": "2026-10-28T15:00:00Z"},
                }));
            then.status(201).json_body(json!({
                "seq": 4, "kind": "message", "from_agent_id": "me",
                "text": "Wednesday at 3 works", "data": {"slot": "2026-10-28T15:00:00Z"},
                "blocked": false, "created_at": "2026-10-26T10:02:00Z",
            }));
        })
        .await;
    let history = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/calls/c1/messages")
                .query_param("after", "2")
                .query_param("limit", "10")
                .query_param("q", "wednesday slot");
            then.status(200).json_body(json!([
                {
                    "seq": 3, "kind": "message", "from_agent_id": "caller-agent",
                    "text": "Mail me at [contact removed]", "data": null, "blocked": false,
                    "safety": [{"kind": "email", "action": "redacted", "path": "message.text"}],
                    "created_at": "2026-10-26T10:01:00Z",
                },
                {
                    "seq": 4, "kind": "message", "from_agent_id": "me",
                    "text": "Wednesday at 3 works", "data": {"slot": "2026-10-28T15:00:00Z"},
                    "blocked": false, "created_at": "2026-10-26T10:02:00Z",
                },
                {
                    "seq": 5, "kind": "invocation", "from_agent_id": "me",
                    "text": null, "data": null, "blocked": false,
                    "invocation": {"id": "inv1", "capability": "book_meeting", "status": "succeeded"},
                    "created_at": "2026-10-26T10:03:00Z",
                },
            ]));
        })
        .await;

    let chakra = build(&server);
    let calls = chakra.calls();
    let sent = calls
        .send(
            "c1",
            "me",
            "Wednesday at 3 works",
            Some(json!({"slot": "2026-10-28T15:00:00Z"})),
        )
        .await
        .expect("sends");
    send.assert_async().await;
    assert_eq!((sent.seq, sent.kind), (4, CallEntryKind::Message));

    let entries = calls
        .history(
            "c1",
            CallHistoryQuery {
                after: Some(2),
                limit: Some(10),
                q: Some("wednesday slot".into()),
            },
        )
        .await
        .expect("history");
    history.assert_async().await;
    assert_eq!(entries.len(), 3);
    let finding = &entries[0].safety.as_ref().expect("safety")[0];
    assert_eq!(
        (finding.kind.as_str(), finding.action.as_str()),
        ("email", "redacted")
    );
    assert_eq!(entries[1].text.as_deref(), Some("Wednesday at 3 works"));
    assert_eq!(entries[1].data, sent.data);
    assert_eq!(entries[1].safety, None);
    let inv = entries[2].invocation.as_ref().expect("invocation");
    assert_eq!(entries[2].kind, CallEntryKind::Invocation);
    assert_eq!(inv.capability.as_deref(), Some("book_meeting"));
    assert_eq!(inv.status, Some(InvocationStatus::Succeeded));
}

#[tokio::test]
async fn ask_parses_a_recall_and_memory_a_bundle() {
    let server = MockServer::start_async().await;
    let ask = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/calls/c1/memory/ask")
                .json_body(json!({"agent_id": "me", "question": "When did we agree to meet?"}));
            then.status(200).json_body(json!({
                "answered": 0.93,
                "messages": [
                    {"seq": 4, "from": "me", "text": "Wednesday at 3 works",
                     "at": "2026-10-26T10:02:00Z", "p": 0.88},
                    {"seq": 2, "from": "caller-agent", "text": "How about Wednesday?",
                     "at": "2026-10-26T10:00:30Z", "p": 0.07},
                ],
            }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/calls/c1/memory");
            then.status(200).json_body(json!({
                "pinned": [{
                    "seq": 4, "from": "me", "at": "2026-10-26T10:02:00Z",
                    "text": "Agreed.", "context": "Wednesday at 3?",
                    "tags": {"decision": 0.91, "commitment": 0.4, "open_question": 0.02,
                             "fact": 0.1, "weight": 2.8},
                }],
                "open": [],
                "recent": [{
                    "seq": 5, "from": "me", "at": "2026-10-26T10:03:00Z",
                    "invocation": {"id": "inv1", "capability": "book_meeting",
                                   "status": "succeeded", "output": {"booked": true}},
                }],
            }));
        })
        .await;

    let chakra = build(&server);
    let recall = chakra
        .calls()
        .ask("c1", "me", "When did we agree to meet?")
        .await
        .expect("asks");
    ask.assert_async().await;
    assert_eq!(recall.answered, Some(0.93));
    assert_eq!(recall.messages.len(), 2);
    assert_eq!(recall.messages[0].seq, 4);
    assert_eq!(
        recall.messages[0].text.as_deref(),
        Some("Wednesday at 3 works")
    );
    assert_eq!(recall.messages[0].p, 0.88);

    let memory = chakra.calls().memory("c1").await.expect("memory");
    let pinned = &memory.pinned[0];
    assert_eq!(pinned.context.as_deref(), Some("Wednesday at 3?"));
    assert_eq!(pinned.tags.expect("tags").decision, 0.91);
    assert!(memory.open.is_empty());
    let inv = memory.recent[0].invocation.as_ref().expect("invocation");
    assert_eq!(inv.status, InvocationStatus::Succeeded);
    assert_eq!(inv.output, Some(json!({"booked": true})));
    assert_eq!(inv.input, None);
}

#[tokio::test]
async fn calls_error_code_surfaces() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/calls/c1/messages");
            then.status(409).json_body(json!({
                "error": {"code": "chk.calls.not_active", "message": "this call isn't active"},
            }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/calls/c1/memory/ask");
            then.status(429).json_body(json!({
                "error": {"code": "chk.calls.slow_down", "message": "at most 30 questions an hour"},
            }));
        })
        .await;

    let chakra = build(&server);
    match chakra.calls().send("c1", "me", "hello?", None).await {
        Err(Error::Api {
            status,
            code,
            message,
            ..
        }) => {
            assert_eq!(status, 409);
            assert_eq!(code, "chk.calls.not_active");
            assert_eq!(message, "this call isn't active");
        }
        other => panic!("expected Api, got {other:?}"),
    }
    match chakra.calls().ask("c1", "me", "why?").await {
        Err(Error::Api { status, code, .. }) => {
            assert_eq!((status, code.as_str()), (429, "chk.calls.slow_down"));
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

// ─── serve ───────────────────────────────────────────────

#[tokio::test]
async fn serve_handles_each_kind_and_survives_handler_errors() {
    let server = MockServer::start_async().await;
    let mut with_memory = item_json("c-reply", 3, "message", Some("ping"));
    with_memory["memory"] = json!({
        "pinned": [], "open": [],
        "recent": [{"seq": 3, "from": "caller-agent", "at": "2026-10-26T10:01:00Z", "text": "ping"}],
    });
    let mut ended = item_json("c-reply", 4, "ended", None);
    ended["call"]["state"] = json!("ended");
    ended["call"]["ended_by"] = json!("caller");
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "me")
                .query_param("limit", "25");
            then.status(200).json_body(json!([
                {"call": call_json("c-invite", "ringing"),
                 "entry": entry_json(1, "invite", Some("Hi, got a minute?"))},
                item_json("c-boom", 2, "message", Some("boom")),
                item_json("c-panic", 2, "message", Some("panic")),
                with_memory,
                item_json("c-gone", 7, "message", Some("late")),
                ended,
            ]));
        })
        .await;
    let accept = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/calls/c-invite/accept")
                .json_body(json!({"agent_id": "me"}));
            then.status(200).json_body(call_json("c-invite", "active"));
        })
        .await;
    let reply = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/calls/c-reply/messages")
                .json_body(json!({"agent_id": "me", "text": "pong"}));
            then.status(201)
                .json_body(entry_json(5, "message", Some("pong")));
        })
        .await;
    let gone = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/calls/c-gone/messages");
            then.status(409).json_body(json!({
                "error": {"code": "chk.calls.not_active", "message": "this call isn't active"},
            }));
        })
        .await;

    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let seen = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let events = Arc::new(Mutex::new(Vec::<(String, CallEntryKind)>::new()));
    let errors = Arc::new(Mutex::new(Vec::<CallsServeError>::new()));

    let serve = {
        let seen = seen.clone();
        let events = events.clone();
        let errors = errors.clone();
        let cancel_on_event = cancel.clone();
        chakra
            .calls()
            .serve("me")
            .on_message(move |item| {
                let seen = seen.clone();
                async move {
                    let text = item.entry.text.clone().unwrap_or_default();
                    seen.lock()
                        .unwrap()
                        .push((text.clone(), item.memory.is_some()));
                    match text.as_str() {
                        "boom" => Err("handler failed".to_string()),
                        "panic" => panic!("handler panicked"),
                        "ping" => Ok(Some(CallReply::from("pong"))),
                        _ => Ok(Some(CallReply::text("too late"))),
                    }
                }
            })
            .on_event(move |item| {
                let events = events.clone();
                let cancel = cancel_on_event.clone();
                async move {
                    events.lock().unwrap().push((item.call.id, item.entry.kind));
                    // Stop once this batch is handled.
                    cancel.cancel();
                    Ok::<_, std::convert::Infallible>(())
                }
            })
            .on_error(move |e| errors.lock().unwrap().push(e))
            .poll_interval(Duration::from_millis(20))
            .with_cancellation(cancel.clone())
    };
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .expect("serve stops when cancelled")
        .expect("serve completes");

    // The invite was accepted by default, once: the loop stopped after
    // the batch.
    assert_eq!(feed.calls_async().await, 1);
    assert_eq!(accept.calls_async().await, 1);
    // The message after the failing handlers was still answered, and the
    // reply to the call that ended meanwhile was refused quietly.
    assert_eq!(reply.calls_async().await, 1);
    assert_eq!(gone.calls_async().await, 1);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            ("boom".to_string(), false),
            ("panic".to_string(), false),
            ("ping".to_string(), true),
            ("late".to_string(), false),
        ]
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![("c-reply".to_string(), CallEntryKind::Ended)]
    );
    let errors = errors.lock().unwrap();
    assert_eq!(errors.len(), 2, "got {errors:?}");
    for (e, (want_call, want_message)) in errors.iter().zip([
        ("c-boom", "handler failed"),
        ("c-panic", "handler panicked"),
    ]) {
        match e {
            CallsServeError::Handler {
                handler,
                call_id,
                message,
                ..
            } => {
                assert_eq!(*handler, "on_message");
                assert_eq!(call_id, want_call);
                assert!(message.contains(want_message), "got {message}");
            }
            other => panic!("expected a handler error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn serve_on_invite_can_decline() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/inbox/calls");
            then.status(200).json_body(json!([
                {"call": call_json("c-invite", "ringing"),
                 "entry": entry_json(1, "invite", Some("Buy my course?"))},
            ]));
        })
        .await;
    let accept = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/calls/c-invite/accept");
            then.status(200).json_body(call_json("c-invite", "active"));
        })
        .await;
    let decline = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/calls/c-invite/decline")
                .json_body(json!({"agent_id": "me", "reason": "not interested"}));
            then.status(200)
                .json_body(call_json("c-invite", "declined"));
        })
        .await;

    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let cancel_on_invite = cancel.clone();
    let serve = chakra
        .calls()
        .serve("me")
        .on_invite(move |item| {
            let cancel = cancel_on_invite.clone();
            async move {
                assert_eq!(item.entry.kind, CallEntryKind::Invite);
                assert_eq!(item.entry.text.as_deref(), Some("Buy my course?"));
                cancel.cancel();
                Ok::<_, std::convert::Infallible>(InviteAnswer::decline_with("not interested"))
            }
        })
        .with_cancellation(cancel.clone());
    tokio::time::timeout(Duration::from_secs(5), serve)
        .await
        .expect("serve stops when cancelled")
        .expect("serve completes");

    assert_eq!(decline.calls_async().await, 1);
    assert_eq!(accept.calls_async().await, 0);
}

#[tokio::test]
async fn serve_stops_when_cancelled_while_idle() {
    let server = MockServer::start_async().await;
    let feed = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/inbox/calls")
                .query_param("agent_id", "me");
            then.status(200).json_body(json!([]));
        })
        .await;

    let chakra = build(&server);
    let cancel = CancellationToken::new();
    let serve = chakra
        .calls()
        .serve("me")
        // Far longer than the test: only the token can end the sleep.
        .poll_interval(Duration::from_secs(60))
        .with_cancellation(cancel.clone());
    let canceller = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
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

// ─── call_id and accept_calls on existing routes ─────────

#[tokio::test]
async fn invoke_sends_call_id() {
    let server = MockServer::start_async().await;
    let invoke = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/invoke").json_body(json!({
                "capability_id": "cap",
                "grantee_agent_id": "me",
                "input": {"day": "wednesday"},
                "call_id": "c1",
            }));
            then.status(200).json_body(json!({
                "invocation_id": "inv1", "status": "pending", "error": null,
            }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/invocations/inv1");
            then.status(200).json_body(json!({
                "id": "inv1", "grant_id": null, "granter_agent_id": "caller-agent",
                "granter_display_name": null, "grantee_agent_id": "me",
                "grantee_display_name": null, "capability_id": "cap",
                "capability_name": "book_meeting", "status": "pending", "elapsed_ms": 0,
                "error_message": null, "input_preview": {"day": "wednesday"},
                "output_preview": null, "created_at": "2026-10-26T10:03:00Z",
                "claimed_at": null, "call_id": "c1", "i_served": false, "i_invoked": true,
            }));
        })
        .await;

    let chakra = build(&server);
    let enq = chakra
        .invoke(&InvokeRequest {
            capability_id: Some("cap".into()),
            grantee_agent_id: "me".into(),
            input: json!({"day": "wednesday"}),
            call_id: Some("c1".into()),
            ..Default::default()
        })
        .await
        .expect("invokes");
    invoke.assert_async().await;
    let inv = chakra
        .invocations()
        .get(&enq.invocation_id)
        .await
        .expect("get");
    assert_eq!(inv.call_id.as_deref(), Some("c1"));
}

#[tokio::test]
async fn agents_carry_and_update_accept_calls() {
    let server = MockServer::start_async().await;
    let agent = |accept_calls: Option<&str>| {
        let mut a = json!({
            "id": "a1", "account_id": "acc-2", "account_slug": "mine",
            "account_display_name": "Mine", "slug": "helper", "display_name": "Helper",
            "description": "", "visibility": "network", "endpoint_url": null,
            "created_at": "2026-10-26T10:00:00Z", "updated_at": "2026-10-26T10:00:00Z",
            "is_mine": true, "capability_count": 1, "avg_rating": null, "review_count": 0,
        });
        if let Some(v) = accept_calls {
            a["accept_calls"] = json!(v);
        }
        a
    };
    let patch = server
        .mock_async(|when, then| {
            when.method(PATCH)
                .path("/v1/agents/a1")
                .json_body(json!({"accept_calls": "anyone"}));
            then.status(200).json_body(agent(Some("anyone")));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/agents/old");
            then.status(200).json_body(agent(None));
        })
        .await;

    let chakra = build(&server);
    let updated = chakra
        .agents()
        .update(
            "a1",
            &UpdateAgentRequest {
                accept_calls: Some(AcceptCalls::Anyone),
                ..Default::default()
            },
        )
        .await
        .expect("updates");
    patch.assert_async().await;
    assert_eq!(updated.accept_calls, Some(AcceptCalls::Anyone));
    // A relay that predates calls doesn't send it.
    let old = chakra.agents().get("old").await.expect("get");
    assert_eq!(old.accept_calls, None);
}
