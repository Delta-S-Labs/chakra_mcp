//! A2A JSON-RPC + streaming endpoints with auth + policy gate (D4).
//!
//! D2 publishes Agent Cards whose `supported_interfaces[].url` points
//! at these routes. This handler runs the full policy decision tree
//! before responding:
//!
//! - Deny branches return JSON-RPC 2.0 error envelopes with the
//!   stable `data.code` from the discovery design's error catalog.
//!   Generic A2A clients learn the failure reason machine-readably.
//! - The success branch still returns 501 — D5 lands the actual
//!   forward (JWT-mint + proxy for push, inbox-bridge park for pull).

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::agent_card::keys::KeyStore;
use crate::forwarder::{forward_push, ForwardError, ForwardOutcome};
use crate::inbox_bridge::{
    get_task, park, GetTaskError, ParkConsent, ParkError, CONSENT_TERMS_METADATA_KEY,
    SAFETY_METADATA_KEY,
};
use crate::policy::{evaluate, Authorized, Decision, DenyReason};
use crate::state::RelayState;

/// `POST /agents/<account_slug>/<agent_slug>/a2a/jsonrpc`
///
/// Dispatches on JSON-RPC `method`:
/// - `SendMessage` → policy gate + forward (push) / park (pull).
/// - `tasks/get`   → return Task wrapping the parked invocation row.
/// - anything else → method-not-found JSON-RPC error.
pub async fn jsonrpc_stub(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.config.discovery_v2_enabled {
        return discovery_disabled();
    }

    // Peek at method + id without committing to a full envelope shape
    // (clients may add fields we don't model).
    let envelope: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return parse_error(),
    };
    let method = envelope.get("method").and_then(|v| v.as_str());
    let req_id = envelope
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    match method {
        Some("SendMessage") => {
            handle_send_message(&state, &account_slug, &agent_slug, &headers, body).await
        }
        Some("tasks/get") => handle_get_task(&state, &headers, envelope, req_id).await,
        Some(other) => method_not_found(other, req_id),
        None => parse_error(),
    }
}

/// Run the policy gate, then forward (push) or park (pull).
async fn handle_send_message(
    state: &RelayState,
    account_slug: &str,
    agent_slug: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let decision = evaluate(&state.db, headers, state, account_slug, agent_slug).await;
    match decision {
        Decision::Authorized(authz) => {
            // Usage limits (per-account rate + credits), keyed on the caller's
            // account. In-memory only: the credits worker keeps the switch
            // current. In shadow mode a would-block is logged and the call
            // proceeds; when enforcing, it's refused (→ HTTP 429).
            let limits = state.enforce_limits(authz.caller_account_id, "a2a").await;
            if let Some(outcome) = limits.refusal {
                return limit_deny_response(outcome, state.credits_url.as_deref());
            }
            let ctx = match compliance_context(&state.db, &authz).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "capability lookup failed post-authorization");
                    return internal_error();
                }
            };
            // System One compliance (when enabled), judged on the A2A
            // `params` (the message being sent). Fail-open lives inside
            // the checker; only an actual violation lands here. With
            // message safety on, screening replaces the check and may
            // rewrite the body (contacts removed, findings in metadata).
            let mut body = body;
            let mut trust_snapshot = None;
            let mut safety_metadata = None;
            if let Some(checker) = state.compliance.as_deref() {
                let envelope: serde_json::Value =
                    serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                let input = envelope.get("params").unwrap_or(&envelope);
                let subject = crate::compliance::Subject {
                    capability_name: &ctx.name,
                    capability_description: Some(ctx.description.as_str())
                        .filter(|d| !d.trim().is_empty()),
                    grant_purpose: ctx.grant_purpose.as_deref(),
                    relationship: Some(crate::compliance::Relationship {
                        proposer_message: ctx.proposer_message.as_deref(),
                        response_message: ctx.response_message.as_deref(),
                    }),
                    input,
                };
                if checker.safety_enabled() {
                    match screen_a2a(checker, subject, &envelope, &body).await {
                        Err(response) => {
                            tracing::info!(
                                grant_id = %authz.grant_id,
                                capability = %ctx.name,
                                "A2A call refused by message safety screening"
                            );
                            return response;
                        }
                        Ok(screened) => {
                            body = screened.body;
                            trust_snapshot = Some(screened.trust_snapshot);
                            safety_metadata = screened.metadata;
                        }
                    }
                } else {
                    match checker.check(&subject).await {
                        crate::compliance::Verdict::Deny { reason, report } => {
                            tracing::info!(
                                grant_id = %authz.grant_id,
                                capability = %ctx.name,
                                %report,
                                "A2A call denied by System One compliance check"
                            );
                            return compliance_deny_response(&reason);
                        }
                        crate::compliance::Verdict::Allow { report } => {
                            trust_snapshot = Some(serde_json::json!({ "system_one": report }));
                        }
                    }
                }
            }
            let cap_name = ctx.name;
            // Push agents can't require consent (owner consent §11), so the
            // gate only applies to parked calls.
            let consent = ctx.consent_required.then_some(ParkConsent {
                max_validity_secs: ctx.consent_max_validity_secs,
                wait_secs: ctx.consent_wait_secs,
            });
            let mut response = if authz.target_is_push {
                handle_authorized_push(
                    state,
                    authz,
                    cap_name,
                    Some(ctx.description).filter(|d| !d.trim().is_empty()),
                    body,
                    headers,
                    trust_snapshot,
                )
                .await
            } else {
                handle_authorized_pull(
                    state,
                    authz,
                    cap_name,
                    body,
                    consent,
                    trust_snapshot,
                    safety_metadata,
                )
                .await
            };
            limits.signals.apply(response.headers_mut());
            response
        }
        Decision::Denied(reason) => deny_response(&reason),
    }
}

/// Caller polls for completion of a task they previously submitted.
/// Auth: same bearer + caller-agent header as SendMessage. The
/// caller_agent must equal the invocation's grantee_agent_id —
/// strangers can't probe other agents' tasks.
async fn handle_get_task(
    state: &RelayState,
    headers: &HeaderMap,
    envelope: serde_json::Value,
    req_id: serde_json::Value,
) -> Response {
    // Reuse the same identity-resolution rules as the policy gate.
    // We don't run the FULL policy gate here (no friendship/grant
    // check needed — those were enforced when the task was parked)
    // but we do need to verify the bearer + caller-agent ownership.
    let caller_agent_id = match crate::policy::evaluator::resolve_caller_agent_for_get_task(
        &state.db, headers, state,
    )
    .await
    {
        Ok(id) => id,
        Err(reason) => return deny_response(&reason),
    };

    // Extract task id from params.
    let task_id_str = envelope
        .get("params")
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str());
    let Some(task_id_str) = task_id_str else {
        return jsonrpc_error_with_id(
            req_id,
            -32602,
            "tasks/get requires params.id",
            Some(serde_json::json!({"code":"chk.request.invalid_params"})),
        );
    };
    let Ok(task_id) = task_id_str.parse::<uuid::Uuid>() else {
        return jsonrpc_error_with_id(
            req_id,
            -32602,
            "tasks/get params.id is not a valid UUID",
            Some(serde_json::json!({"code":"chk.request.invalid_params"})),
        );
    };

    match get_task(&state.db, &state.heartbeats, task_id, caller_agent_id).await {
        Ok(task) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": task,
            })),
        )
            .into_response(),
        Err(GetTaskError::NotFound) => jsonrpc_error_with_id(
            req_id,
            -32006,
            "task not found",
            Some(serde_json::json!({"code":"chk.target.not_found"})),
        ),
        Err(GetTaskError::NotYourTask) => jsonrpc_error_with_id(
            req_id,
            -32003,
            "task not owned by caller",
            Some(serde_json::json!({"code":"chk.policy.task_not_yours"})),
        ),
        Err(GetTaskError::Db(e)) => {
            tracing::error!(error = %e, "get_task DB error");
            internal_error()
        }
    }
}

fn parse_error() -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32700,
                "message": "parse error",
                "data": { "code": "chk.request.parse_error" }
            }
        })),
    )
        .into_response()
}

fn method_not_found(method: &str, req_id: serde_json::Value) -> Response {
    jsonrpc_error_with_id(
        req_id,
        -32601,
        &format!("method '{method}' not supported"),
        Some(serde_json::json!({"code":"chk.request.unknown_method","method":method})),
    )
}

fn jsonrpc_error_with_id(
    req_id: serde_json::Value,
    code: i32,
    message: &str,
    data: Option<serde_json::Value>,
) -> Response {
    let http = match code {
        -32700 | -32600 | -32602 => StatusCode::BAD_REQUEST,
        -32601 => StatusCode::NOT_IMPLEMENTED,
        -32003 => StatusCode::FORBIDDEN,
        -32006 => StatusCode::NOT_FOUND,
        _ => StatusCode::OK,
    };
    let mut error = serde_json::json!({"code": code, "message": message});
    if let Some(d) = data {
        error["data"] = d;
    }
    (
        http,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": error,
        })),
    )
        .into_response()
}

/// `POST /agents/<account_slug>/<agent_slug>/a2a/stream`
pub async fn stream_stub(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !state.config.discovery_v2_enabled {
        return discovery_disabled();
    }
    // Streaming uses the same gate; the post-pass SSE-passthrough
    // implementation is its own design (mid-stream consent expiry,
    // cancellation, replica failover) and lands later in D5+.
    match evaluate(&state.db, &headers, &state, &account_slug, &agent_slug).await {
        Decision::Authorized(_) => not_implemented(),
        Decision::Denied(reason) => deny_response(&reason),
    }
}

/// Forward an authorized push call upstream and return the response.
#[allow(clippy::too_many_arguments)]
async fn handle_authorized_push(
    state: &RelayState,
    authz: Authorized,
    capability_name: String,
    capability_description: Option<String>,
    body: Bytes,
    headers: &HeaderMap,
    trust_snapshot: Option<serde_json::Value>,
) -> Response {
    // Message safety (when on) screens the agent's response too.
    let reply_screen = state
        .compliance
        .as_deref()
        .filter(|c| c.safety_enabled())
        .map(|checker| crate::forwarder::ReplyScreen {
            checker,
            capability_name: &capability_name,
            capability_description: capability_description.as_deref(),
        });
    let keystore = KeyStore::new(state.db.clone());

    match forward_push(
        &state.db,
        &keystore,
        &state.forward_http,
        state.outbound,
        &authz,
        &capability_name,
        body,
        headers,
        trust_snapshot,
        reply_screen,
    )
    .await
    {
        Ok(ForwardOutcome {
            http_status,
            body,
            content_type,
        }) => {
            let mut h = HeaderMap::new();
            h.insert(
                header::CONTENT_TYPE,
                content_type
                    .as_deref()
                    .and_then(|s| axum::http::HeaderValue::from_str(s).ok())
                    .unwrap_or_else(|| axum::http::HeaderValue::from_static("application/json")),
            );
            (
                StatusCode::from_u16(http_status).unwrap_or(StatusCode::OK),
                h,
                body,
            )
                .into_response()
        }
        Err(
            e @ (ForwardError::NoCachedCard
            | ForwardError::NoUpstreamInterface
            | ForwardError::Blocked(_)),
        ) => {
            if let ForwardError::Blocked(msg) = &e {
                tracing::warn!(error = %msg, agent_id = %authz.target_agent_id, "push forward refused");
            }
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "application/json")],
                Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32005,
                        "message": "target agent's upstream card not cached or unreachable",
                        "data": { "code": "chk.target.unreachable" }
                    }
                })),
            )
                .into_response()
        }
        Err(ForwardError::Transport(msg)) => {
            tracing::warn!(error = %msg, "upstream forward transport error");
            (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32005,
                        "message": "upstream agent unreachable",
                        "data": { "code": "chk.target.unreachable" }
                    }
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "forward_push internal error");
            internal_error()
        }
    }
}

/// Park an authorized pull-mode call and return an A2A Task with
/// `state: "submitted"` so the caller can begin polling via D5d's
/// GetTask handler. The granter SDK's `inbox.serve()` will pick the
/// row up on its next poll of `/v1/inbox` and post the result back
/// via `/v1/invocations/{id}/result` — both endpoints predate the
/// migration and survive unchanged.
async fn handle_authorized_pull(
    state: &RelayState,
    authz: Authorized,
    capability_name: String,
    body: Bytes,
    consent: Option<ParkConsent>,
    trust_snapshot: Option<serde_json::Value>,
    safety_metadata: Option<serde_json::Value>,
) -> Response {
    match park(
        &state.db,
        &state.wake,
        &authz,
        &capability_name,
        body,
        consent,
        trust_snapshot,
    ).await {
        Ok(mut parked) => {
            // Parked for a pull agent: whether it's around (waking pull
            // agents §8).
            let peer = state.heartbeats.peer(authz.target_agent_id, "pull");
            parked.task.metadata = crate::inbox_bridge::with_peer(safety_metadata, Some(peer));
            (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": parked.jsonrpc_id,
                "result": parked.task,
            })),
        )
            .into_response()
        }
        Err(ParkError::BodyTooLarge(n)) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32600,
                    "message": "request body too large to park",
                    "data": { "code": "chk.request.too_large", "byte_count": n }
                }
            })),
        )
            .into_response(),
        Err(ParkError::BadConsentTerms(detail)) => (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32602,
                    "message": format!("bad consent terms in params.metadata[\"{CONSENT_TERMS_METADATA_KEY}\"]: {detail}"),
                    "data": { "code": "chk.consent.bad_terms" }
                }
            })),
        )
            .into_response(),
        Err(ParkError::Db(e)) => {
            tracing::error!(error = %e, "park failed");
            internal_error()
        }
        Err(ParkError::Enqueue(e)) => {
            tracing::error!(error = %e, "park (consent) failed");
            internal_error()
        }
    }
}

/// What the forwarder + the compliance check need about the authorised
/// capability, grant and friendship, in one round trip.
struct ComplianceContext {
    name: String,
    description: String,
    grant_purpose: Option<String>,
    proposer_message: Option<String>,
    response_message: Option<String>,
    consent_required: bool,
    consent_max_validity_secs: i32,
    consent_wait_secs: i32,
}

async fn compliance_context(
    db: &sqlx::PgPool,
    authz: &Authorized,
) -> Result<ComplianceContext, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT cap.name, cap.description,
               cap.consent_required, cap.consent_max_validity_secs, cap.consent_wait_secs,
               g.purpose           as "grant_purpose?",
               f.proposer_message  as "proposer_message?",
               f.response_message  as "response_message?"
        FROM agent_capabilities cap
        LEFT JOIN grants g ON g.id = $2
        LEFT JOIN LATERAL (
            SELECT proposer_message, response_message
            FROM friendships
            WHERE status = 'accepted'
              AND ((proposer_agent_id = $3 AND target_agent_id = $4)
                OR (proposer_agent_id = $4 AND target_agent_id = $3))
            ORDER BY decided_at DESC
            LIMIT 1
        ) f ON true
        WHERE cap.id = $1
        "#,
        authz.capability_id,
        authz.grant_id,
        authz.caller_agent_id,
        authz.target_agent_id,
    )
    .fetch_one(db)
    .await?;
    Ok(ComplianceContext {
        name: row.name,
        description: row.description,
        grant_purpose: row.grant_purpose,
        proposer_message: row.proposer_message,
        response_message: row.response_message,
        consent_required: row.consent_required,
        consent_max_validity_secs: row.consent_max_validity_secs,
        consent_wait_secs: row.consent_wait_secs,
    })
}

/// Like [`deny_response`] for `ComplianceDenied`, but carries the
/// checker's per-question detail in `data.detail`.
fn compliance_deny_response(detail: &str) -> Response {
    let reason = DenyReason::ComplianceDenied;
    (
        jsonrpc_to_http(reason.jsonrpc_code()),
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": reason.jsonrpc_code(),
                "message": reason.message(),
                "data": { "code": reason.data_code(), "detail": detail }
            }
        })),
    )
        .into_response()
}

/// A screened A2A message: the body to deliver (rewritten when something
/// was removed or flagged), its `trust_snapshot`, and the task metadata.
struct ScreenedA2a {
    body: Bytes,
    trust_snapshot: serde_json::Value,
    metadata: Option<serde_json::Value>,
}

/// Screen an A2A message's `params` (message safety S2). The capability's
/// input schema doesn't describe the A2A envelope, so none is used. A body
/// that isn't JSON is screened as text. `Err` is the response to send.
async fn screen_a2a(
    checker: &crate::compliance::ComplianceChecker,
    subject: crate::compliance::Subject<'_>,
    envelope: &serde_json::Value,
    body: &Bytes,
) -> Result<ScreenedA2a, Response> {
    use chakramcp_shared::safety::Channel;
    let parsed = !envelope.is_null();
    let text = serde_json::Value::String(String::from_utf8_lossy(body).into_owned());
    let (input, root) = if parsed {
        (subject.input, "params")
    } else {
        (&text, "body")
    };
    let subject = crate::compliance::Subject { input, ..subject };
    let screened = checker
        .screen(Channel::Request, &subject, root, None, "")
        .await
        .map_err(|_| safety_too_large_response())?;
    let trust_snapshot = serde_json::json!({ "system_one": { "safety": screened.report } });
    if let Some(block) = screened.block {
        return Err(safety_block_response(block.code, &block.reason));
    }
    if screened.findings.is_empty() {
        return Ok(ScreenedA2a {
            body: body.clone(),
            trust_snapshot,
            metadata: None,
        });
    }
    let found = serde_json::json!({ "request": screened.findings });
    let body = if parsed {
        let mut envelope = envelope.clone();
        let mut params = screened.value;
        if let Some(p) = params.as_object_mut() {
            let metadata = p.entry("metadata").or_insert_with(|| serde_json::json!({}));
            if let Some(m) = metadata.as_object_mut() {
                m.insert(SAFETY_METADATA_KEY.into(), found.clone());
            }
        }
        envelope["params"] = params;
        Bytes::from(serde_json::to_vec(&envelope).unwrap_or_default())
    } else {
        Bytes::from(screened.value.as_str().unwrap_or_default().to_owned())
    };
    Ok(ScreenedA2a {
        body,
        trust_snapshot,
        metadata: Some(serde_json::json!({ SAFETY_METADATA_KEY: found })),
    })
}

/// Message safety refused the call: today's `-32009` with the safety code.
fn safety_block_response(code: &str, detail: &str) -> Response {
    let reason = DenyReason::ComplianceDenied;
    (
        jsonrpc_to_http(reason.jsonrpc_code()),
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": reason.jsonrpc_code(),
                "message": reason.message(),
                "data": { "code": code, "detail": detail }
            }
        })),
    )
        .into_response()
}

/// A message over the screening cap: refused, never screened in part.
fn safety_too_large_response() -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32600,
                "message": "message too large to screen",
                "data": { "code": "chk.safety.too_large" }
            }
        })),
    )
        .into_response()
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
}

/// A usage-limit refusal: the policy deny shape, plus when to retry (in
/// `Retry-After` and `data`) and, for credits, the next grant and where to
/// buy (chakramcp.com only).
fn limit_deny_response(
    outcome: crate::limits::LimitOutcome,
    credits_url: Option<&str>,
) -> Response {
    use crate::limits::LimitOutcome;
    let (reason, retry_after_secs, extra) = match outcome {
        LimitOutcome::RateLimited {
            limit,
            retry_after_secs,
        } => (
            DenyReason::RateLimited,
            retry_after_secs,
            serde_json::json!({ "limit": limit }),
        ),
        LimitOutcome::InsufficientCredits {
            retry_after_secs,
            next_grant_at,
        } => {
            let mut extra = serde_json::json!({ "next_grant_at": next_grant_at });
            if let Some(url) = credits_url {
                extra["credits_url"] = serde_json::json!(url);
            }
            (DenyReason::InsufficientCredits, retry_after_secs, extra)
        }
    };
    let mut data = serde_json::json!({
        "code": reason.data_code(),
        "retry_after_secs": retry_after_secs,
    });
    if let (Some(data), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
        data.extend(extra.clone());
    }
    let mut response = (
        jsonrpc_to_http(reason.jsonrpc_code()),
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": reason.jsonrpc_code(),
                "message": reason.message(),
                "data": data,
            }
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, retry_after_secs.into());
    response
}

fn deny_response(reason: &DenyReason) -> Response {
    let http_status = jsonrpc_to_http(reason.jsonrpc_code());
    (
        http_status,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": reason.jsonrpc_code(),
                "message": reason.message(),
                "data": { "code": reason.data_code() }
            }
        })),
    )
        .into_response()
}

fn not_implemented() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32601,
                "message": "method not implemented yet",
                "data": { "code": "chk.not_implemented_yet", "ships_in": "D5" }
            }
        })),
    )
        .into_response()
}

fn discovery_disabled() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "error": "A2A endpoint not enabled",
            "code": "chk.discovery_v2_disabled",
        })),
    )
        .into_response()
}

/// Map our JSON-RPC error code family to an appropriate HTTP status.
/// Conventions:
/// - -32000 (auth missing) → 401
/// - -32001 (auth invalid) → 401
/// - -32002 (friendship) / -32003 (grant) / -32009 (compliance) → 403
/// - -32005 (unreachable) → 503
/// - -32006 (target tombstoned/missing) → 404
/// - -32007 (rate) / -32008 (credits) → 429
fn jsonrpc_to_http(code: i32) -> StatusCode {
    match code {
        -32000 | -32001 => StatusCode::UNAUTHORIZED,
        -32002 | -32003 | -32009 | -32010 => StatusCode::FORBIDDEN,
        -32005 => StatusCode::SERVICE_UNAVAILABLE,
        -32006 => StatusCode::NOT_FOUND,
        -32007 | -32008 => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::OK,
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests cover every branch of the decision tree
    //! end-to-end via the production router. Each test seeds DB
    //! state precisely to land in a specific branch, fires a real
    //! HTTP request, and asserts the response code + data.code.

    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config_v2_on() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: true,
            log_filter: "warn".into(),
        }
    }

    /// Standard fixture: two accounts, two agents (one per account),
    /// a capability on the target, an accepted friendship, an active
    /// grant. Returns ids the tests can manipulate to land in
    /// specific deny branches.
    struct Fixture {
        caller_user_id: Uuid,
        caller_account_id: Uuid,
        caller_agent_id: Uuid,
        target_account_slug: String,
        target_account_id: Uuid,
        target_agent_slug: String,
        target_agent_id: Uuid,
        capability_id: Uuid,
        api_key_plaintext: String,
    }

    async fn seed_full_fixture(pool: &PgPool) -> Fixture {
        // Caller user + their api key
        let caller_user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Caller', 'x')"#,
            caller_user_id,
            format!("caller-{caller_user_id}@test.local"),
        )
        .execute(pool)
        .await
        .unwrap();

        let api_key_plaintext = format!("ck_test_{caller_user_id}");
        let mut hasher = Sha256::new();
        hasher.update(api_key_plaintext.as_bytes());
        let key_hash = hex::encode(hasher.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'test', 'ck_test')"#,
            Uuid::now_v7(),
            caller_user_id,
            key_hash,
        )
        .execute(pool)
        .await
        .unwrap();

        // Caller account + membership
        let caller_account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Caller Org', 'individual', $3)"#,
            caller_account_id,
            format!("caller-acct-{caller_account_id}"),
            caller_user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            caller_account_id,
            caller_user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        let caller_agent_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'caller-agent', 'Caller Agent', 'network')"#,
            caller_agent_id,
            caller_account_id,
        )
        .execute(pool)
        .await
        .unwrap();

        // Target account + agent + capability
        let target_account_id = Uuid::now_v7();
        let target_account_slug = format!("target-acct-{target_account_id}");
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Target Org', 'individual')"#,
            target_account_id,
            target_account_slug,
        )
        .execute(pool)
        .await
        .unwrap();
        let target_agent_id = Uuid::now_v7();
        let target_agent_slug = "target-agent".to_string();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, $3, 'Target Agent', 'network')"#,
            target_agent_id,
            target_account_id,
            target_agent_slug,
        )
        .execute(pool)
        .await
        .unwrap();
        let capability_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb, 'network')"#,
            capability_id,
            target_agent_id,
        )
        .execute(pool)
        .await
        .unwrap();

        // Friendship (accepted) + Grant (active, target → caller, on this capability)
        sqlx::query!(
            r#"INSERT INTO friendships
                  (id, proposer_agent_id, target_agent_id, status, decided_at)
               VALUES ($1, $2, $3, 'accepted', now())"#,
            Uuid::now_v7(),
            target_agent_id,
            caller_agent_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO grants
                  (id, granter_agent_id, grantee_agent_id, capability_id, status)
               VALUES ($1, $2, $3, $4, 'active')"#,
            Uuid::now_v7(),
            target_agent_id,
            caller_agent_id,
            capability_id,
        )
        .execute(pool)
        .await
        .unwrap();

        Fixture {
            caller_user_id,
            caller_account_id,
            caller_agent_id,
            target_account_slug,
            target_account_id,
            target_agent_slug,
            target_agent_id,
            capability_id,
            api_key_plaintext,
        }
    }

    use sha2::{Digest, Sha256};

    fn req(
        path: &str,
        bearer: Option<&str>,
        caller_agent: Option<&str>,
        capability: Option<&str>,
    ) -> Request<Body> {
        // A valid JSON-RPC SendMessage envelope so the handler's
        // method-dispatch path resolves to the policy gate. Tests
        // that need a different body (tasks/get, malformed) build
        // their own request directly.
        let mut b = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = bearer {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if let Some(c) = caller_agent {
            b = b.header("X-ChakraMCP-Caller-Agent", c);
        }
        if let Some(c) = capability {
            b = b.header("X-ChakraMCP-Capability", c);
        }
        b.body(Body::from(
            br#"{"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{}}"#.to_vec(),
        ))
        .unwrap()
    }

    async fn parse_body(res: Response) -> serde_json::Value {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn path_for(f: &Fixture) -> String {
        format!(
            "/agents/{}/{}/a2a/jsonrpc",
            f.target_account_slug, f.target_agent_slug
        )
    }

    // ── Auth branches ─────────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_auth_missing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(&path_for(&f), None, None, None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32000);
        assert_eq!(body["error"]["data"]["code"], "chk.auth.missing");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_auth_invalid(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(&path_for(&f), Some("ck_not_a_real_key"), None, None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["data"]["code"], "chk.auth.invalid");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_caller_agent_header_missing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(&path_for(&f), Some(&f.api_key_plaintext), None, None))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.auth.caller_agent_header_missing"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_caller_agent_not_owned(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        // Provide a UUID that ISN'T one of the caller's agents — use the target's.
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.target_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.auth.caller_agent_not_owned"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_capability_header_missing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.auth.capability_header_missing"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_target_account_not_found(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                "/agents/no-such-account/whatever/a2a/jsonrpc",
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.target.not_found"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_target_tombstoned(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!(
            "UPDATE agents SET tombstoned_at = now() WHERE id = $1",
            f.target_agent_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.target.tombstoned"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_capability_not_on_target(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&Uuid::now_v7().to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.target.capability_unknown"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_friendship_required(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!("DELETE FROM friendships")
            .execute(&pool)
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.policy.friendship_required"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_grant_required(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!("DELETE FROM grants")
            .execute(&pool)
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(
            parse_body(res).await["error"]["data"]["code"],
            "chk.policy.grant_required"
        );
    }

    /// Default fixture is a *pull*-mode target (mode column defaults
    /// to 'pull'). The handler now parks the call into the inbox bridge
    /// and returns an A2A Task with state="submitted" — D5d's GetTask
    /// handler completes the polling loop.
    #[sqlx::test(migrations = "../migrations")]
    async fn pull_pass_parks_and_returns_a2a_task(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path_for(&f))
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", f.api_key_plaintext),
                    )
                    .header("X-ChakraMCP-Caller-Agent", f.caller_agent_id.to_string())
                    .header("X-ChakraMCP-Capability", f.capability_id.to_string())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":7,"method":"SendMessage","params":{"text":"hi"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        // JSON-RPC envelope echoes the original id and embeds the Task.
        assert_eq!(body["id"], 7);
        let task = &body["result"];
        assert!(task["id"].is_string(), "Task.id present");
        assert_eq!(task["status"]["state"], "submitted");

        // Underlying invocation row is pending and assigned to the
        // target as granter (so its inbox.serve loop will pick it up).
        let row = sqlx::query!(
            r#"SELECT status, granter_agent_id, grantee_agent_id, capability_id, input_preview
                 FROM relay_invocations LIMIT 1"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.granter_agent_id, Some(f.target_agent_id));
        assert_eq!(row.grantee_agent_id, Some(f.caller_agent_id));
        assert_eq!(row.capability_id, Some(f.capability_id));
        // input_preview holds SendMessage params — the granter SDK's
        // existing `inbox.serve()` handler sees the same shape it
        // always saw (backward compat with v0.1.0 SDK contract).
        assert_eq!(row.input_preview, Some(serde_json::json!({"text": "hi"})));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn deny_compliance_check(pool: PgPool) {
        use crate::compliance::{questions, test_support};
        let f = seed_full_fixture(&pool).await;
        let mut fake = test_support::Fake::default();
        fake.nouls.insert(questions::DATA_EXFILTRATION, 0.9);
        let url = test_support::serve(fake.clone()).await;
        let state = crate::state::RelayState::new(pool.clone(), config_v2_on())
            .with_compliance(Some(test_support::checker(&url)));

        let res = crate::router(state)
            .oneshot(a2a_send_request(&f))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32009);
        assert_eq!(
            body["error"]["data"]["code"],
            "chk.policy.compliance_denied"
        );
        assert!(body["error"]["data"]["detail"]
            .as_str()
            .unwrap()
            .contains("data_exfiltration"));

        // Nothing was parked, and the model judged the SendMessage params.
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM relay_invocations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
        let sent = fake.last_body.lock().unwrap().clone().unwrap();
        assert!(sent["state"]["request"]["input"].is_object(), "{sent}");
    }

    // ─── Usage limits: credits ───────────────────────────────

    /// Charge-queue rows written for `account` (one per accepted invocation).
    async fn queued_charges(pool: &PgPool, account: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM credit_charge_queue WHERE account_id = $1")
            .bind(account)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn a2a_send_request(f: &Fixture) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path_for(f))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", f.api_key_plaintext),
            )
            .header("X-ChakraMCP-Caller-Agent", f.caller_agent_id.to_string())
            .header("X-ChakraMCP-Capability", f.capability_id.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","id":7,"method":"SendMessage","params":{"text":"hi"}}"#,
            ))
            .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn shadow_mode_allows_when_out_of_credits(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let credits = crate::limits::credits::exhausted_cache(&pool, f.caller_account_id).await;
        // Default RelayState → limits_enforce = false (shadow mode).
        let app = crate::router(
            crate::state::RelayState::new(pool.clone(), config_v2_on()).with_credit_cache(credits),
        );
        let res = app.oneshot(a2a_send_request(&f)).await.unwrap();
        // Out of credits, but shadow mode logs a would-block and still parks
        // — and still charges.
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(queued_charges(&pool, f.caller_account_id).await, 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn enforcing_refuses_when_out_of_credits(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let credits = crate::limits::credits::exhausted_cache(&pool, f.caller_account_id).await;
        let app = crate::router(
            crate::state::RelayState::new(pool.clone(), config_v2_on())
                .with_limits_enforce(true)
                .with_credit_cache(credits)
                .with_credits_url(Some("https://chakramcp.test/app/credits".into())),
        );
        let res = app.oneshot(a2a_send_request(&f)).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(res.headers().contains_key(header::RETRY_AFTER));
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32008);
        assert_eq!(body["error"]["data"]["code"], "chk.limit.credits");
        assert!(body["error"]["data"]["retry_after_secs"].as_u64().unwrap() >= 1);
        assert!(body["error"]["data"]["next_grant_at"].is_string());
        assert_eq!(
            body["error"]["data"]["credits_url"],
            "https://chakramcp.test/app/credits"
        );
        // Refused before dispatch: nothing parked, nothing charged.
        assert_eq!(queued_charges(&pool, f.caller_account_id).await, 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn an_allowed_a2a_call_reports_a_low_balance(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let cache = crate::limits::CreditCache::default();
        cache.replace_all(
            std::collections::HashSet::new(),
            std::collections::HashSet::from([f.caller_account_id]),
            std::collections::HashMap::new(),
        );
        let app = crate::router(
            crate::state::RelayState::new(pool.clone(), config_v2_on())
                .with_limits_enforce(true)
                .with_credit_cache(std::sync::Arc::new(cache))
                .with_rate_limiter(crate::limits::RateLimiter::Counting(Default::default())),
        );
        let res = app.oneshot(a2a_send_request(&f)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["chakramcp-credits"], "low");
        assert_eq!(res.headers()["ratelimit-limit"], "60");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_pull_invocation_queues_one_charge(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let res = app.oneshot(a2a_send_request(&f)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // The park wrote the invocation and its charge in one statement,
        // against the caller's account.
        assert_eq!(queued_charges(&pool, f.caller_account_id).await, 1);
        let orphaned: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM credit_charge_queue q
              WHERE NOT EXISTS (SELECT 1 FROM relay_invocations i WHERE i.id = q.invocation_id)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            orphaned, 0,
            "every queued charge matches its invocation row"
        );
    }

    /// Promote the fixture's target agent to push mode by giving it
    /// an `agent_card_url` and a cached envelope pointing at the
    /// supplied upstream URL. The policy gate's TargetUnreachable
    /// check is bypassed by setting `agent_card_fetched_at = now()`.
    async fn promote_target_to_push(pool: &PgPool, f: &Fixture, upstream_url: &str) {
        use crate::agent_card::fetcher::CachedCardEnvelope;
        use crate::agent_card::types::{
            AgentCapabilities, AgentCard, AgentInterface, A2A_PROTOCOL_VERSION,
            PROTOCOL_BINDING_JSONRPC,
        };
        use std::collections::BTreeMap;
        let upstream_card = AgentCard {
            name: "Target".into(),
            description: "Test".into(),
            supported_interfaces: vec![AgentInterface {
                url: upstream_url.into(),
                protocol_binding: PROTOCOL_BINDING_JSONRPC.into(),
                tenant: None,
                protocol_version: A2A_PROTOCOL_VERSION.into(),
                extra: Default::default(),
            }],
            provider: None,
            version: "1.0.0".into(),
            documentation_url: None,
            capabilities: AgentCapabilities {
                streaming: Some(false),
                push_notifications: Some(false),
                extensions: vec![],
                extended_agent_card: None,
                extra: Default::default(),
            },
            security_schemes: BTreeMap::new(),
            security_requirements: vec![],
            default_input_modes: vec!["application/json".into()],
            default_output_modes: vec!["application/json".into()],
            skills: vec![],
            signatures: vec![],
            icon_url: None,
            extra: Default::default(),
        };
        let envelope = CachedCardEnvelope {
            normalized: upstream_card.clone(),
            upstream: upstream_card,
            etag: None,
        };
        let envelope_json = serde_json::to_value(&envelope).unwrap();
        sqlx::query!(
            r#"UPDATE agents
                  SET mode = 'push',
                      agent_card_url = $2,
                      agent_card_cached = $3,
                      agent_card_fetched_at = now()
                WHERE id = $1"#,
            f.target_agent_id,
            "https://upstream.example.com/.well-known/agent-card.json",
            envelope_json,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// End-to-end push: D2-cached card, D4 policy gate passes, D5b
    /// forwards through to a real test upstream, response flows back.
    #[sqlx::test(migrations = "../migrations")]
    async fn push_authorized_call_proxies_to_upstream(pool: PgPool) {
        // Stand up an upstream that captures the inbound request +
        // returns a JSON-RPC success response.
        use axum::extract::State as AxumState;
        use axum::http::StatusCode as AxumStatus;
        use axum::response::IntoResponse;
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct UpstreamState {
            last_auth: Arc<Mutex<Option<String>>>,
            hits: Arc<AtomicUsize>,
        }

        async fn upstream_handler(
            AxumState(s): AxumState<UpstreamState>,
            headers: HeaderMap,
            _body: axum::body::Bytes,
        ) -> axum::response::Response {
            s.hits.fetch_add(1, Ordering::SeqCst);
            *s.last_auth.lock().unwrap() = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            (
                AxumStatus::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#.to_vec(),
            )
                .into_response()
        }

        let upstream_state = UpstreamState {
            last_auth: Arc::new(Mutex::new(None)),
            hits: Arc::new(AtomicUsize::new(0)),
        };
        let upstream_app = axum::Router::new()
            .route("/a2a/jsonrpc", post(upstream_handler))
            .with_state(upstream_state.clone());
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_url = format!("http://{}/a2a/jsonrpc", upstream_addr);
        tokio::spawn(async move {
            axum::serve(upstream_listener, upstream_app).await.ok();
        });

        // Seed standard caller/target fixture, then promote target to push.
        let f = seed_full_fixture(&pool).await;
        promote_target_to_push(&pool, &f, &upstream_url).await;

        // Mint an active signing key (forwarder needs it). Normally
        // this happens lazily on first card-fetch; here we ensure
        // it explicitly so JWKS has something for the upstream to
        // verify against if it wanted to.
        let keystore = crate::agent_card::KeyStore::new(pool.clone());
        let _ = keystore.ensure_active_key().await.unwrap();

        // Caller POSTs to our relay's A2A endpoint. The full pipeline
        // runs: D4 policy gate -> D5b forwarder.
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        assert_eq!(body["result"]["ok"], true);

        // Upstream got our request with a relay-minted JWT.
        assert_eq!(upstream_state.hits.load(Ordering::SeqCst), 1);
        let auth = upstream_state.last_auth.lock().unwrap().clone().unwrap();
        let bearer = auth.strip_prefix("Bearer ").unwrap();
        let pub_keys = keystore.jwks_keys().await.unwrap();
        let claims = crate::jwt_mint::decode_relay_jwt(bearer, &pub_keys, chrono::Utc::now())
            .expect("upstream-bound JWT must verify against our JWKS");
        assert_eq!(claims.sub, f.caller_agent_id.to_string());
        assert_eq!(claims.aud, f.target_agent_id.to_string());
        assert_eq!(claims.capability_id, f.capability_id.to_string());

        // Audit row written.
        let row = sqlx::query!("SELECT status, http_status FROM relay_invocations LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.status, "succeeded");
        assert_eq!(row.http_status, Some(200));
        // The forwarder queued the charge with the row, against the caller.
        assert_eq!(queued_charges(&pool, f.caller_account_id).await, 1);
    }

    /// Push-mode call where upstream is unreachable returns
    /// 502 + chk.target.unreachable to the caller (no leaking
    /// "the upstream is at xyz.example.com:1234" detail).
    #[sqlx::test(migrations = "../migrations")]
    async fn push_authorized_call_502s_when_upstream_unreachable(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        promote_target_to_push(&pool, &f, "http://127.0.0.1:1/a2a/jsonrpc").await;
        let _ = crate::agent_card::KeyStore::new(pool.clone())
            .ensure_active_key()
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["data"]["code"], "chk.target.unreachable");
        // Every accepted attempt is charged, failed ones included.
        assert_eq!(queued_charges(&pool, f.caller_account_id).await, 1);
    }

    // ── D5d: tasks/get end-to-end ─────────────────────────

    fn req_with_body(
        path: &str,
        bearer: Option<&str>,
        caller_agent: Option<&str>,
        capability: Option<&str>,
        body: serde_json::Value,
    ) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = bearer {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if let Some(c) = caller_agent {
            b = b.header("X-ChakraMCP-Caller-Agent", c);
        }
        if let Some(c) = capability {
            b = b.header("X-ChakraMCP-Capability", c);
        }
        b.body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    /// A capability that asks its owner first holds the A2A call: the task
    /// starts `auth-required` and stays so on tasks/get, no charge is
    /// queued, and the terms come from `params.metadata` (owner consent
    /// §4.1, §8).
    #[sqlx::test(migrations = "../migrations")]
    async fn consent_required_call_is_held_as_auth_required(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query(
            "UPDATE agent_capabilities
                SET consent_required = true, consent_max_validity_secs = 86400
              WHERE id = $1",
        )
        .bind(f.capability_id)
        .execute(&pool)
        .await
        .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));
        let send = |id: i32, metadata: serde_json::Value| {
            req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "SendMessage",
                    "params": {"text": "book it", "metadata": metadata}
                }),
            )
        };

        let bad = app
            .clone()
            .oneshot(send(
                1,
                serde_json::json!({ CONSENT_TERMS_METADATA_KEY: {"valid_for": "9y"} }),
            ))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            parse_body(bad).await["error"]["data"]["code"],
            "chk.consent.bad_terms"
        );

        let res = app
            .clone()
            .oneshot(send(
                2,
                serde_json::json!({ CONSENT_TERMS_METADATA_KEY: {"valid_for": "30d"} }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        assert_eq!(body["result"]["status"]["state"], "auth-required");
        let task_id = body["result"]["id"].as_str().unwrap().to_string();
        let task_uuid: Uuid = task_id.parse().unwrap();

        let (status, terms): (String, serde_json::Value) = sqlx::query_as(
            "SELECT i.status, cr.terms FROM relay_invocations i
               JOIN consent_requests cr ON cr.invocation_id = i.id WHERE i.id = $1",
        )
        .bind(task_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "awaiting_consent");
        // Capped at the publisher's longest consent.
        assert_eq!(terms["valid_for"], "1d");
        let charged: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM credit_charge_queue WHERE invocation_id = $1)",
        )
        .bind(task_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!charged);

        let polled = app
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None,
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 3, "method": "tasks/get", "params": {"id": task_id}
                }),
            ))
            .await
            .unwrap();
        let polled = parse_body(polled).await;
        assert_eq!(polled["result"]["status"]["state"], "auth-required");
        assert_eq!(
            polled["result"]["status"]["message"],
            "Waiting for the owner's approval"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn parked_task_then_get_task_polls_through_completion(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));

        // 1. Caller submits SendMessage. Pull-mode → parked.
        let send_res = app
            .clone()
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "SendMessage",
                    "params": {"text": "do the thing"}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(send_res.status(), StatusCode::OK);
        let send_body = parse_body(send_res).await;
        let task_id = send_body["result"]["id"].as_str().unwrap().to_string();

        // 2. Caller polls tasks/get → state should be "submitted".
        let get1 = app
            .clone()
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None, // capability not required for tasks/get
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tasks/get",
                    "params": {"id": task_id}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(get1.status(), StatusCode::OK);
        let get1_body = parse_body(get1).await;
        assert_eq!(get1_body["id"], 2);
        assert_eq!(get1_body["result"]["id"], task_id);
        assert_eq!(get1_body["result"]["status"]["state"], "submitted");

        // 3. Granter SDK posts a result via the legacy endpoint.
        // Simulate that by directly updating the row to status='succeeded'
        // with output_preview set — the relay handler does the same thing
        // when it receives POST /v1/invocations/{id}/result.
        let task_uuid: uuid::Uuid = task_id.parse().unwrap();
        sqlx::query!(
            r#"UPDATE relay_invocations
                  SET status = 'succeeded',
                      output_preview = '{"slots":[1,2,3]}'::jsonb
                WHERE id = $1"#,
            task_uuid,
        )
        .execute(&pool)
        .await
        .unwrap();

        // 4. Caller polls again → state="completed", artifacts carry output.
        let get2 = app
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tasks/get",
                    "params": {"id": task_id}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(get2.status(), StatusCode::OK);
        let get2_body = parse_body(get2).await;
        assert_eq!(get2_body["result"]["status"]["state"], "completed");
        let arts = get2_body["result"]["artifacts"].as_array().unwrap();
        assert_eq!(arts.len(), 1);
        assert_eq!(
            arts[0]["parts"][0]["data"],
            serde_json::json!({"slots":[1,2,3]})
        );
    }

    /// While a parked task is open its metadata says whether the target is
    /// around (waking pull agents §8); once it settles, it doesn't.
    #[sqlx::test(migrations = "../migrations")]
    async fn open_tasks_carry_peer(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let state = crate::state::RelayState::new(pool.clone(), config_v2_on());
        let app = crate::router(state.clone());
        let send = |id: i32, method: &str, params: serde_json::Value, capability: bool| {
            req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                capability.then(|| f.capability_id.to_string()).as_deref(),
                serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
            )
        };

        let sent = parse_body(
            app.clone()
                .oneshot(send(1, "SendMessage", serde_json::json!({}), true))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            sent["result"]["metadata"]["chakramcp.peer"],
            serde_json::json!({ "delivery": "pull", "online": false, "last_seen_at": null }),
            "{sent}"
        );
        let task_id = sent["result"]["id"].as_str().unwrap().to_string();

        state.heartbeats.record(f.target_agent_id);
        let get = || send(2, "tasks/get", serde_json::json!({ "id": task_id }), false);
        let polled = parse_body(app.clone().oneshot(get()).await.unwrap()).await;
        assert_eq!(
            polled["result"]["metadata"]["chakramcp.peer"]["online"], true,
            "{polled}"
        );

        sqlx::query("UPDATE relay_invocations SET status = 'succeeded' WHERE id = $1")
            .bind(task_id.parse::<Uuid>().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let done = parse_body(app.oneshot(get()).await.unwrap()).await;
        assert_eq!(done["result"]["status"]["state"], "completed");
        assert!(done["result"].get("metadata").is_none(), "{done}");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn get_task_returns_failed_state_with_error_message(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));

        // Park.
        let r = app
            .clone()
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
                serde_json::json!({"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{}}),
            ))
            .await
            .unwrap();
        let body = parse_body(r).await;
        let task_id = body["result"]["id"].as_str().unwrap().to_string();
        let task_uuid: uuid::Uuid = task_id.parse().unwrap();

        // Granter reports failure.
        sqlx::query!(
            r#"UPDATE relay_invocations
                  SET status = 'failed',
                      error_message = 'capability raised'
                WHERE id = $1"#,
            task_uuid,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Poll.
        let res = app
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None,
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 2,
                    "method": "tasks/get",
                    "params": {"id": task_id}
                }),
            ))
            .await
            .unwrap();
        let body = parse_body(res).await;
        assert_eq!(body["result"]["status"]["state"], "failed");
        assert_eq!(body["result"]["status"]["message"], "capability raised");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn get_task_rejects_caller_who_didnt_originate_it(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config_v2_on()));

        // Park as the original caller.
        let r = app
            .clone()
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
                serde_json::json!({"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{}}),
            ))
            .await
            .unwrap();
        let task_id = parse_body(r).await["result"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        // Build a SECOND user who has no relationship to this task.
        let other_user = uuid::Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Other', 'x')"#,
            other_user,
            format!("other-{other_user}@t.local"),
        )
        .execute(&pool)
        .await
        .unwrap();
        let other_key = format!("ck_other_{other_user}");
        let mut h = Sha256::new();
        h.update(other_key.as_bytes());
        let kh = hex::encode(h.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'k', 'ck_other')"#,
            uuid::Uuid::now_v7(),
            other_user,
            kh,
        )
        .execute(&pool)
        .await
        .unwrap();
        let other_account = uuid::Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Other', 'individual', $3)"#,
            other_account,
            format!("oa-{other_account}"),
            other_user,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            uuid::Uuid::now_v7(),
            other_account,
            other_user,
        )
        .execute(&pool)
        .await
        .unwrap();
        let other_agent = uuid::Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name)
               VALUES ($1, $2, 'eve', 'Eve')"#,
            other_agent,
            other_account,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Other user tries to read the task.
        let res = app
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&other_key),
                Some(&other_agent.to_string()),
                None,
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 9,
                    "method": "tasks/get",
                    "params": {"id": task_id}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["data"]["code"], "chk.policy.task_not_yours");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn unknown_method_returns_method_not_found(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req_with_body(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                None,
                serde_json::json!({
                    "jsonrpc": "2.0", "id": 1,
                    "method": "UnknownThing",
                    "params": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32601);
        assert_eq!(body["error"]["data"]["code"], "chk.request.unknown_method");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn malformed_json_body_returns_parse_error(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path_for(&f))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", f.api_key_plaintext),
                    )
                    .header("X-ChakraMCP-Caller-Agent", f.caller_agent_id.to_string())
                    .body(Body::from(b"not valid json".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32700);
    }

    /// Sanity that the deny path doesn't smuggle any caller info
    /// into the response payload — phishing-resistant by spec.
    #[sqlx::test(migrations = "../migrations")]
    async fn error_response_carries_no_user_info(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        sqlx::query!("DELETE FROM friendships")
            .execute(&pool)
            .await
            .unwrap();
        let app = crate::router(crate::state::RelayState::new(pool, config_v2_on()));
        let res = app
            .oneshot(req(
                &path_for(&f),
                Some(&f.api_key_plaintext),
                Some(&f.caller_agent_id.to_string()),
                Some(&f.capability_id.to_string()),
            ))
            .await
            .unwrap();
        let body = parse_body(res).await;
        let body_str = serde_json::to_string(&body).unwrap();
        // Don't surface user ids, account ids, or agent UUIDs in the
        // response. The catalog (D12) provides any deep-links by code.
        assert!(!body_str.contains(&f.caller_user_id.to_string()));
        assert!(!body_str.contains(&f.caller_account_id.to_string()));
        assert!(!body_str.contains(&f.caller_agent_id.to_string()));
        assert!(!body_str.contains(&f.target_account_id.to_string()));
        assert!(!body_str.contains(&f.target_agent_id.to_string()));
    }

    // ─── Message safety (S2) ─────────────────────────────────

    fn send_text(f: &Fixture, text: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path_for(f))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", f.api_key_plaintext),
            )
            .header("X-ChakraMCP-Caller-Agent", f.caller_agent_id.to_string())
            .header("X-ChakraMCP-Capability", f.capability_id.to_string())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "jsonrpc": "2.0", "id": 7, "method": "SendMessage",
                    "params": { "text": text },
                }))
                .unwrap(),
            ))
            .unwrap()
    }

    async fn safety_state(
        pool: &PgPool,
        nouls: &[(&'static str, f64)],
        safety: bool,
    ) -> crate::state::RelayState {
        use crate::compliance::test_support;
        let fake = test_support::Fake {
            nouls: nouls.iter().copied().collect(),
            ..Default::default()
        };
        let url = test_support::serve(fake).await;
        crate::state::RelayState::new(pool.clone(), config_v2_on())
            .with_compliance(Some(test_support::checker(&url).with_safety(safety)))
    }

    async fn only_row(pool: &PgPool) -> (Uuid, serde_json::Value, serde_json::Value) {
        let (id, input, snap): (Uuid, serde_json::Value, Option<serde_json::Value>) =
            sqlx::query_as("SELECT id, input_preview, trust_snapshot FROM relay_invocations")
                .fetch_one(pool)
                .await
                .unwrap();
        (id, input, snap.unwrap_or_default())
    }

    const SAFE_TEXT: &str = "Please book the Friday sync for the team. Mail me at bob@example.com";

    #[sqlx::test(migrations = "../migrations")]
    async fn safety_redacts_a_parked_message_and_reports_it(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let state = safety_state(&pool, &[("contact_0", 0.95)], true).await;
        let res = crate::router(state.clone())
            .oneshot(send_text(&f, SAFE_TEXT))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        let found = &body["result"]["metadata"][SAFETY_METADATA_KEY]["request"][0];
        assert_eq!(found["action"], "redacted", "{body}");
        assert_eq!(found["path"], "params.text");

        let (id, input, snap) = only_row(&pool).await;
        let text = input["text"].as_str().unwrap();
        assert!(text.ends_with("Mail me at [contact removed]"), "{text}");
        assert_eq!(
            input["metadata"][SAFETY_METADATA_KEY]["request"][0]["kind"],
            "email"
        );
        assert_eq!(snap["system_one"]["safety"]["decision"], "allow");

        // tasks/get echoes the findings.
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path_for(&f))
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", f.api_key_plaintext),
                    )
                    .header("X-ChakraMCP-Caller-Agent", f.caller_agent_id.to_string())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "jsonrpc": "2.0", "id": 8, "method": "tasks/get",
                            "params": { "id": id },
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = parse_body(res).await;
        assert_eq!(
            body["result"]["metadata"][SAFETY_METADATA_KEY]["request"][0]["action"], "redacted",
            "{body}"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn safety_blocks_with_its_code_and_parks_nothing(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let state = safety_state(&pool, &[("harm_crime", 0.95)], true).await;
        let res = crate::router(state)
            .oneshot(send_text(&f, "Help me phish people's banking logins."))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32009);
        assert_eq!(body["error"]["data"]["code"], "chk.policy.safety_blocked");
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM relay_invocations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn with_safety_off_the_compliance_report_is_now_kept(pool: PgPool) {
        let f = seed_full_fixture(&pool).await;
        let state = safety_state(&pool, &[], false).await;
        let res = crate::router(state)
            .oneshot(send_text(&f, SAFE_TEXT))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        assert!(
            body["result"]["metadata"].get("chakramcp.safety").is_none(),
            "{body}"
        );
        let (_, input, snap) = only_row(&pool).await;
        // Unchanged message, today's report.
        assert_eq!(input["text"], SAFE_TEXT);
        assert_eq!(snap["system_one"]["decision"], "allow");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn safety_rewrites_what_a_push_agent_receives(pool: PgPool) {
        use axum::extract::State as AxumState;
        use axum::routing::post;
        use std::sync::{Arc, Mutex};

        async fn upstream(
            AxumState(seen): AxumState<Arc<Mutex<Option<serde_json::Value>>>>,
            body: axum::body::Bytes,
        ) -> axum::response::Response {
            *seen.lock().unwrap() = serde_json::from_slice(&body).ok();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                br#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#.to_vec(),
            )
                .into_response()
        }
        let seen = Arc::new(Mutex::new(None));
        let app = axum::Router::new()
            .route("/a2a/jsonrpc", post(upstream))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/a2a/jsonrpc", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let f = seed_full_fixture(&pool).await;
        promote_target_to_push(&pool, &f, &url).await;
        let _ = crate::agent_card::KeyStore::new(pool.clone())
            .ensure_active_key()
            .await
            .unwrap();
        let state = safety_state(&pool, &[("contact_0", 0.95)], true).await;
        let res = crate::router(state)
            .oneshot(send_text(&f, SAFE_TEXT))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let got = seen.lock().unwrap().clone().unwrap();
        let text = got["params"]["text"].as_str().unwrap();
        assert!(text.ends_with("Mail me at [contact removed]"), "{got}");
        assert_eq!(
            got["params"]["metadata"][SAFETY_METADATA_KEY]["request"][0]["action"],
            "redacted"
        );
        let (_, _, snap) = only_row(&pool).await;
        assert_eq!(snap["system_one"]["safety"]["decision"], "allow");
    }

    /// A push upstream answering every call with `reply`.
    async fn push_upstream(reply: &'static str) -> String {
        use axum::routing::post;
        async fn handler(
            axum::extract::State(reply): axum::extract::State<&'static str>,
        ) -> axum::response::Response {
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                reply.as_bytes().to_vec(),
            )
                .into_response()
        }
        let app = axum::Router::new()
            .route("/a2a/jsonrpc", post(handler))
            .with_state(reply);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/a2a/jsonrpc", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        url
    }

    async fn push_fixture(pool: &PgPool, reply: &'static str) -> Fixture {
        let f = seed_full_fixture(pool).await;
        promote_target_to_push(pool, &f, &push_upstream(reply).await).await;
        let _ = crate::agent_card::KeyStore::new(pool.clone())
            .ensure_active_key()
            .await
            .unwrap();
        f
    }

    /// Safety on, with the request's call answered clean and the reply's
    /// with `reply_nouls`.
    async fn push_safety_state(
        pool: &PgPool,
        reply_nouls: &[(&'static str, f64)],
    ) -> crate::state::RelayState {
        use crate::compliance::test_support;
        let fake = test_support::Fake::default();
        fake.per_call.lock().unwrap().push_back(Default::default());
        fake.per_call
            .lock()
            .unwrap()
            .push_back(reply_nouls.iter().copied().collect());
        let url = test_support::serve(fake).await;
        crate::state::RelayState::new(pool.clone(), config_v2_on())
            .with_compliance(Some(test_support::checker(&url).with_safety(true)))
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn safety_rewrites_a_push_agents_response(pool: PgPool) {
        let f = push_fixture(
            &pool,
            r#"{"jsonrpc":"2.0","id":7,"result":{"text":"Done. Call me on +91 98450 12345."}}"#,
        )
        .await;
        let state = push_safety_state(&pool, &[("contact_0", 0.95)]).await;
        let res = crate::router(state)
            .oneshot(send_text(&f, "Please book the Friday sync for the team."))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = parse_body(res).await;
        assert_eq!(
            body["result"]["text"], "Done. Call me on [contact removed].",
            "{body}"
        );
        assert_eq!(
            body["result"]["metadata"][SAFETY_METADATA_KEY]["reply"][0]["action"],
            "redacted"
        );
        let (output, snap): (serde_json::Value, Option<serde_json::Value>) =
            sqlx::query_as("SELECT output_preview, trust_snapshot FROM relay_invocations")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            output["result"]["text"],
            "Done. Call me on [contact removed]."
        );
        assert_eq!(
            snap.unwrap()["system_one"]["reply_safety"]["decision"],
            "allow"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn safety_refuses_a_harmful_push_response(pool: PgPool) {
        let f = push_fixture(
            &pool,
            r#"{"jsonrpc":"2.0","id":7,"result":{"text":"Here's how to break into the account."}}"#,
        )
        .await;
        let state = push_safety_state(&pool, &[("harm_malware", 0.95)]).await;
        let res = crate::router(state)
            .oneshot(send_text(&f, "Please book the Friday sync for the team."))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = parse_body(res).await;
        assert_eq!(body["error"]["code"], -32009);
        assert_eq!(body["error"]["data"]["code"], "chk.policy.safety_blocked");
        let (status, output): (String, serde_json::Value) =
            sqlx::query_as("SELECT status, output_preview FROM relay_invocations")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(output, serde_json::json!({ "__chakramcp_blocked__": true }));
    }
}
