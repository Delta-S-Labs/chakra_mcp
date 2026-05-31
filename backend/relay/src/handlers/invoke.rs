//! Async, inbox-pull invocations — the v0.1.0 SDK contract surface.
//!
//! These endpoints predate the A2A migration and are preserved
//! unchanged on `relay.chakramcp.com` per discovery spec Override #2
//! (the "two-host surface model"). The published `chakramcp` Python
//! SDK + `@chakramcp/sdk` TypeScript package at v0.1.0 use exactly
//! these URLs.
//!
//! POST /v1/invoke
//!   Grantee asks the relay to deliver `input` to the granter for
//!   capability `C`. The relay validates the grant, enqueues a row in
//!   `pending`, and returns the invocation id immediately. No outbound
//!   HTTP — the granter side will pull it from their inbox.
//!
//! GET /v1/inbox?agent_id=…&limit=…
//!   The granter's owner pulls oldest pending invocations targeting the
//!   named agent and atomically marks them `in_progress` (so two
//!   concurrent pollers don't claim the same row). They then run the
//!   work locally and report the result.
//!
//! POST /v1/invocations/{id}/result
//!   The granter side reports succeeded or failed with output / error.
//!   Only the user who claimed the row (or another member of the
//!   granter's account) can post a result, and only while it's
//!   in_progress.
//!
//! GET /v1/invocations / GET /v1/invocations/{id}
//!   Audit + status polling. The grantee polls /{id} until status is
//!   terminal (succeeded | failed | timeout | rejected), then reads
//!   output_preview / error_message off the row.
//!
//! ── Relationship to the A2A migration (D6) ─────────────────────
//!
//! These endpoints share the `relay_invocations` table with the new
//! A2A surfaces (D5):
//!
//! - `inbox_bridge::park` (D5c) inserts the same `status='pending'`
//!   shape this `/v1/inbox` polls. A v0.1.0 granter SDK transparently
//!   picks up calls originating from generic A2A clients hitting
//!   `/agents/<acct>/<slug>/a2a/jsonrpc` — no SDK upgrade needed.
//! - `inbox_bridge::get_task` (D5d) reads the same row from the
//!   caller's side and shapes it as an A2A Task. The same row is
//!   both an "Invocation" (legacy poll) and a "Task" (A2A poll).
//! - `forwarder::forward_push` (D5b) writes a terminal-status row
//!   directly. v0.1.0 SDKs invoking via `/v1/invoke` against push
//!   targets currently see a pending row that no one delivers —
//!   push agents are reached only via the new A2A endpoint in v1.
//!   The scheduler-demo (and every v0.1.0 SDK use case to date)
//!   targets pull-mode agents and is unaffected.
//!
//! Regression coverage: `legacy_v01_contract_tests` at the bottom of
//! this module exercises the full v0.1.0 lifecycle on the post-D5
//! schema as the D6 acceptance gate.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

const PREVIEW_BYTE_LIMIT: usize = 16 * 1024;
const INBOX_DEFAULT_LIMIT: i64 = 25;
const INBOX_MAX_LIMIT: i64 = 100;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InvokeRequest {
    /// Trusted path: caller has an active grant from the granter for
    /// the capability. Resolves friendship + grant the existing way.
    /// Mutually exclusive with `capability_id`.
    pub grant_id: Option<Uuid>,
    /// Public path (no friendship/grant required): target the
    /// capability directly by id. Only works for capabilities whose
    /// owner has set `public_invoke = true` (migration 0022). Mutually
    /// exclusive with `grant_id`. The invoker is still authenticated
    /// and must be a member of `grantee_agent_id`'s account; only the
    /// friendship+grant gate is dropped.
    pub capability_id: Option<Uuid>,
    /// The agent the caller is invoking AS — must be a member of its
    /// account. On the trusted path, must match the grant's
    /// grantee_agent_id; on the public path, just identifies the
    /// invoker for audit + quota + later review attribution.
    pub grantee_agent_id: Uuid,
    pub input: Value,
}

#[derive(Debug, Serialize)]
pub struct InvokeResponse {
    pub invocation_id: Uuid,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InvocationDto {
    pub id: Uuid,
    pub grant_id: Option<Uuid>,
    pub granter_agent_id: Option<Uuid>,
    pub granter_display_name: Option<String>,
    pub grantee_agent_id: Option<Uuid>,
    pub grantee_display_name: Option<String>,
    pub capability_id: Option<Uuid>,
    pub capability_name: String,
    /// Capability's HITL classification — `"autonomous"` (worker may
    /// answer directly) or `"human_in_loop"` (worker must defer to
    /// the human owner; the relay's `report_result` gate rejects
    /// non-human-confirmed responses on these). None only if the
    /// invocation has a NULL `capability_id` (orphaned row); SDK
    /// clients should treat None as `"autonomous"` for safety.
    pub semantics: Option<String>,
    pub status: String,
    pub elapsed_ms: i32,
    pub error_message: Option<String>,
    pub input_preview: Option<Value>,
    pub output_preview: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    /// True when the requesting user is on the granter side.
    pub i_served: bool,
    /// True when the requesting user is on the grantee side.
    pub i_invoked: bool,
    /// Trust context bundled in by the relay on inbox.pull responses
    /// only — these fields are populated when the relay just verified
    /// friendship + grant before delivering this row, so the receiving
    /// agent doesn't need to re-query. Always None on list/get
    /// (audit-log) endpoints, where the friendship/grant state at
    /// query time may differ from when the invocation was authorised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendship_context: Option<FriendshipContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_context: Option<GrantContext>,
    /// Which trust path authorised this invocation, computed at read
    /// time from the row's `grant_id`:
    ///
    /// * `"friend"` — `grant_id IS NOT NULL`. The call rode a
    ///   friendship + grant; `grant_context` / `friendship_context`
    ///   are populated on inbox responses.
    /// * `"public"` — `grant_id IS NULL`. The call rode a
    ///   `public_invoke=true` capability (migration 0022); the
    ///   relay's per-invoker quota check is the trust assertion.
    ///   Both contexts come back `None` on inbox responses.
    ///
    /// Use this instead of inspecting `grant_id` / contexts directly —
    /// it removes the "null means public" footgun for handlers and
    /// gives the audit log a single, stable column to filter by.
    pub tier: String,
}

/// Friendship details the relay verified before queuing this
/// invocation. Trust the assertions here without re-querying.
#[derive(Debug, Serialize, Deserialize)]
pub struct FriendshipContext {
    pub id: Uuid,
    pub status: String,
    pub proposer_agent_id: Uuid,
    pub target_agent_id: Uuid,
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// Grant details the relay verified before queuing this invocation.
#[derive(Debug, Serialize, Deserialize)]
pub struct GrantContext {
    pub id: Uuid,
    pub status: String,
    pub granter_agent_id: Uuid,
    pub grantee_agent_id: Uuid,
    pub capability_id: Uuid,
    pub capability_name: String,
    pub capability_visibility: String,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// "outbound" (I invoked — call I sent out)
    /// | "inbound" (I served — call that came in)
    /// | omitted for both.
    ///
    /// Mirrors the convention used by `/v1/friendships` and `/v1/grants`:
    /// "outbound" always means "I initiated this side of the
    /// relationship". For invocations the initiator is the grantee
    /// (the caller).
    pub direction: Option<String>,
    pub agent_id: Option<Uuid>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InboxQuery {
    pub agent_id: Uuid,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ResultRequest {
    /// "succeeded" or "failed".
    pub status: String,
    pub output: Option<Value>,
    pub error: Option<String>,
    /// HITL gate (issue #69 PR 2): when the invocation's capability has
    /// `semantics = 'human_in_loop'`, the relay refuses the result
    /// unless this flag is `true`. The CLI's `inbox respond` + new
    /// `message reply` sugar set it automatically (they assume a human
    /// at the terminal); the Python/TS SDKs default to `false`, which
    /// catches autonomous handlers that try to answer a HITL invocation.
    #[serde(default)]
    pub confirmed_by_human: Option<bool>,
}

// ─── Helpers ─────────────────────────────────────────────

fn truncate_for_audit(value: &Value) -> Value {
    let s = value.to_string();
    if s.len() <= PREVIEW_BYTE_LIMIT {
        value.clone()
    } else {
        serde_json::json!({
            "__chakramcp_truncated__": true,
            "original_byte_length": s.len(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_terminal(
    db: &PgPool,
    grant_id: Option<Uuid>,
    granter_agent_id: Option<Uuid>,
    grantee_agent_id: Option<Uuid>,
    capability_id: Option<Uuid>,
    capability_name: &str,
    invoked_by_user_id: Uuid,
    status: &str,
    elapsed_ms: i32,
    error_message: Option<&str>,
    input_preview: Option<&Value>,
    api_key_id: Option<Uuid>,
    minted_jti: Option<Uuid>,
) -> Result<Uuid, ApiError> {
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, elapsed_ms,
             error_message, input_preview, api_key_id, minted_jti)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
        id,
        grant_id,
        granter_agent_id,
        grantee_agent_id,
        capability_id,
        capability_name,
        invoked_by_user_id,
        status,
        elapsed_ms,
        error_message,
        input_preview.cloned().unwrap_or(Value::Null),
        api_key_id,
        minted_jti,
    )
    .execute(db)
    .await?;
    Ok(id)
}

// ─── POST /v1/invoke (enqueue) ───────────────────────────
//
// Two paths:
//
//   * Trusted (existing): body carries `grant_id`. Resolves friendship +
//     grant exactly as before.
//   * Public (new — migration 0022): body carries `capability_id`. The
//     capability's owner has opted into `public_invoke=true`, so no
//     friendship/grant is required — just authentication + membership
//     in the invoker's own account + a per-invoker monthly quota.
//
// Exactly one of grant_id / capability_id must be set; both, or neither,
// is a 400. Return type widens to `Response` so the public path can emit
// a 429 with a structured quota payload that doesn't fit `InvokeResponse`.
pub async fn invoke(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<InvokeRequest>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;

    let grant_id = match (req.grant_id, req.capability_id) {
        (Some(_), Some(_)) => {
            return Err(ApiError::InvalidRequest(
                "specify exactly one of grant_id or capability_id, not both".into(),
            ));
        }
        (None, None) => {
            return Err(ApiError::InvalidRequest(
                "one of grant_id (trusted) or capability_id (public) is required".into(),
            ));
        }
        (None, Some(capability_id)) => {
            return invoke_public(
                &state,
                &user,
                capability_id,
                req.grantee_agent_id,
                &req.input,
            )
            .await;
        }
        (Some(grant_id), None) => grant_id,
    };

    invoke_trusted(state, user, grant_id, req)
        .await
        .map(IntoResponse::into_response)
}

async fn invoke_trusted(
    state: RelayState,
    user: AuthUser,
    grant_id: Uuid,
    req: InvokeRequest,
) -> Result<(StatusCode, Json<InvokeResponse>), ApiError> {
    let input_preview = truncate_for_audit(&req.input);

    // Resolve the grant + agents + capability.
    let row = sqlx::query!(
        r#"
        SELECT
            g.id as grant_id, g.status as grant_status,
            g.granter_agent_id, g.grantee_agent_id, g.capability_id,
            g.expires_at, g.granted_at,
            ga.account_id as granter_account_id,
            ea.account_id as grantee_account_id,
            cap.name as capability_name,
            cap.visibility as capability_visibility,
            -- Friendship row that authorised this grant. LEFT JOIN so a
            -- grant whose friendship was deleted (shouldn't happen in
            -- normal operation; FKs are tombstone, not cascade) still
            -- resolves — we just record a NULL-friendship snapshot.
            f.id as "friendship_id?",
            f.status as "friendship_status?",
            f.proposer_agent_id as "friendship_proposer_agent_id?",
            f.target_agent_id as "friendship_target_agent_id?",
            f.proposer_message as "friendship_proposer_message?",
            f.response_message as "friendship_response_message?",
            f.decided_at as "friendship_decided_at?"
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        LEFT JOIN friendships f
            ON f.status = 'accepted'
            AND (
                (f.proposer_agent_id = g.granter_agent_id
                    AND f.target_agent_id = g.grantee_agent_id)
                OR (f.proposer_agent_id = g.grantee_agent_id
                    AND f.target_agent_id = g.granter_agent_id)
            )
        WHERE g.id = $1
        LIMIT 1
        "#,
        grant_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.grantee_agent_id != req.grantee_agent_id {
        let id = record_terminal(
            &state.db,
            Some(row.grant_id),
            Some(row.granter_agent_id),
            Some(row.grantee_agent_id),
            Some(row.capability_id),
            &row.capability_name,
            user.user_id,
            "rejected",
            0,
            Some("grantee_agent_id does not match the grant"),
            Some(&input_preview),
            user.api_key_id,
            user.minted_jti,
        )
        .await?;
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(InvokeResponse {
                invocation_id: id,
                status: "rejected".into(),
                error: Some("grantee_agent_id does not match the grant".into()),
            }),
        ));
    }

    // Caller must be a member of the grantee's account.
    if !user_is_member(&state.db, user.user_id, row.grantee_account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Grant must be active and not expired.
    if row.grant_status != "active" {
        let msg = format!(
            "grant is {}; only active grants can be invoked",
            row.grant_status
        );
        let id = record_terminal(
            &state.db,
            Some(row.grant_id),
            Some(row.granter_agent_id),
            Some(row.grantee_agent_id),
            Some(row.capability_id),
            &row.capability_name,
            user.user_id,
            "rejected",
            0,
            Some(&msg),
            Some(&input_preview),
            user.api_key_id,
            user.minted_jti,
        )
        .await?;
        return Ok((
            StatusCode::CONFLICT,
            Json(InvokeResponse {
                invocation_id: id,
                status: "rejected".into(),
                error: Some(msg),
            }),
        ));
    }
    if let Some(exp) = row.expires_at {
        if exp <= Utc::now() {
            let msg = "grant has expired".to_string();
            let id = record_terminal(
                &state.db,
                Some(row.grant_id),
                Some(row.granter_agent_id),
                Some(row.grantee_agent_id),
                Some(row.capability_id),
                &row.capability_name,
                user.user_id,
                "rejected",
                0,
                Some(&msg),
                Some(&input_preview),
                user.api_key_id,
                user.minted_jti,
            )
            .await?;
            return Ok((
                StatusCode::CONFLICT,
                Json(InvokeResponse {
                    invocation_id: id,
                    status: "rejected".into(),
                    error: Some(msg),
                }),
            ));
        }
    }

    // Freeze the trust context at queue time (migration 0024). Audit
    // log endpoints return this verbatim so they show what was true
    // when the call happened, not what's true now. Shape mirrors the
    // typed FriendshipContext + GrantContext so consumers see one shape
    // across inbox + audit-log responses.
    let trust_snapshot = serde_json::json!({
        "friendship": row.friendship_id.map(|fid| serde_json::json!({
            "id": fid,
            "status": row.friendship_status.clone().unwrap_or_default(),
            "proposer_agent_id": row.friendship_proposer_agent_id,
            "target_agent_id": row.friendship_target_agent_id,
            "proposer_message": row.friendship_proposer_message,
            "response_message": row.friendship_response_message,
            "decided_at": row.friendship_decided_at,
        })),
        "grant": {
            "id": row.grant_id,
            "status": row.grant_status,
            "granter_agent_id": row.granter_agent_id,
            "grantee_agent_id": row.grantee_agent_id,
            "capability_id": row.capability_id,
            "capability_name": row.capability_name.clone(),
            "capability_visibility": row.capability_visibility,
            "granted_at": row.granted_at,
            "expires_at": row.expires_at,
        },
    });

    // Enqueue the invocation. Granter side will pull it from /v1/inbox.
    //
    // `api_key_id` attributes the call to the **caller** — the credential
    // that authed *this* POST /v1/invoke request. The result-post on the
    // other side reuses this row (UPDATE only), so this is the only
    // place per invocation where the caller's credential is recorded.
    // NULL on the JWT path (web session / OAuth / device flow).
    //
    // `minted_jti` is the dual: NULL on the ck_ path, set on the JWT
    // path so the per-pair dashboard can attribute the call back to
    // the device-flow / oauth-code row that minted this token.
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, input_preview,
             api_key_id, minted_jti, trust_snapshot)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9, $10, $11)
        "#,
        id,
        row.grant_id,
        row.granter_agent_id,
        row.grantee_agent_id,
        row.capability_id,
        row.capability_name,
        user.user_id,
        input_preview,
        user.api_key_id,
        user.minted_jti,
        trust_snapshot,
    )
    .execute(&state.db)
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(InvokeResponse {
            invocation_id: id,
            status: "pending".into(),
            error: None,
        }),
    ))
}

/// Public-invoke path (migration 0022). Caller addresses the capability
/// directly — no grant, no friendship. Still must authenticate + own
/// the invoker agent. Enforces a per-invoker monthly quota counted
/// against `relay_invocations`. Writes the invocation with
/// `grant_id = NULL`.
async fn invoke_public(
    state: &RelayState,
    user: &AuthUser,
    capability_id: Uuid,
    grantee_agent_id: Uuid,
    input: &Value,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let input_preview = truncate_for_audit(input);

    // Resolve the capability + its owning agent. We hide existence of
    // non-public or non-network capabilities with a uniform 404 so this
    // endpoint can't be used to probe private state.
    let cap = sqlx::query!(
        r#"
        SELECT cap.id, cap.name, cap.agent_id AS granter_agent_id,
               cap.public_invoke, cap.public_monthly_quota_per_agent,
               ga.visibility AS granter_visibility
        FROM agent_capabilities cap
        JOIN agents ga ON ga.id = cap.agent_id
        WHERE cap.id = $1
        "#,
        capability_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !cap.public_invoke || cap.granter_visibility != "network" {
        return Err(ApiError::NotFound);
    }
    let quota = cap.public_monthly_quota_per_agent.ok_or_else(|| {
        // Belt: the CHECK guarantees this, but if a row ever slipped
        // through it'd be a real misconfig and the right move is 500.
        ApiError::Internal(anyhow::anyhow!(
            "public_invoke=true with NULL quota for capability {}; DB CHECK should have prevented this",
            capability_id
        ))
    })?;

    // Resolve the invoker agent + its account, then membership-check.
    let invoker = sqlx::query!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        grantee_agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::InvalidRequest("grantee_agent_id not found".into()))?;
    if !user_is_member(&state.db, user.user_id, invoker.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Self-invoke guard. Calling your own agent's public capability
    // wouldn't be wrong per se, but it pollutes quota + future ratings
    // with self-traffic, so we reject it explicitly.
    if cap.granter_agent_id == grantee_agent_id {
        return Err(ApiError::InvalidRequest(
            "cannot invoke your own agent's capability via the public path".into(),
        ));
    }

    // Per-invoker monthly quota. Calendar month (resets on the 1st).
    // Consumed on enqueue: every attempt counts so failures can't be
    // farmed for extra calls. Known race: COUNT-then-INSERT isn't
    // atomic, so concurrent invokes from the same caller can overshoot
    // by a small margin — accepted for v1 (abuse bound, not billing).
    let used: i64 = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "n!"
        FROM relay_invocations
        WHERE grantee_agent_id = $1
          AND capability_id = $2
          AND created_at >= date_trunc('month', now())
        "#,
        grantee_agent_id,
        capability_id,
    )
    .fetch_one(&state.db)
    .await?;
    if used >= i64::from(quota) {
        let resets_at = next_month_start();
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": {
                    "code": "monthly_quota_exhausted",
                    "message": format!(
                        "monthly quota of {quota} public invocations on this capability exhausted; resets at {resets_at}"
                    ),
                    "retryable": false,
                },
                "quota": quota,
                "resets_at": resets_at,
            })),
        )
            .into_response());
    }

    // Enqueue with grant_id = NULL. Same audit columns as the trusted
    // path; the friendship/grant context bundled into the eventual
    // inbox-pull row will be `null` (the inbox handler tolerates it).
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, input_preview,
             api_key_id, minted_jti)
        VALUES ($1, NULL, $2, $3, $4, $5, $6, 'pending', $7, $8, $9)
        "#,
        id,
        cap.granter_agent_id,
        grantee_agent_id,
        capability_id,
        cap.name,
        user.user_id,
        input_preview,
        user.api_key_id,
        user.minted_jti,
    )
    .execute(&state.db)
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(InvokeResponse {
            invocation_id: id,
            status: "pending".into(),
            error: None,
        }),
    )
        .into_response())
}

/// First instant of next calendar month, UTC. Used as `resets_at` in
/// the 429 quota-exhausted response.
fn next_month_start() -> DateTime<Utc> {
    use chrono::{Datelike, NaiveDate, TimeZone};
    let now = Utc::now();
    let (y, m) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(y, m, 1)
            .expect("valid date")
            .and_hms_opt(0, 0, 0)
            .expect("valid time"),
    )
}

// ─── GET /v1/inbox?agent_id=X[&limit=N] ──────────────────
pub async fn inbox(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<InboxQuery>,
) -> ApiResult<Json<Vec<InvocationDto>>> {
    let limit = q
        .limit
        .unwrap_or(INBOX_DEFAULT_LIMIT)
        .clamp(1, INBOX_MAX_LIMIT);

    // Caller must be a member of the agent's account.
    let agent_account =
        sqlx::query_scalar!(r#"SELECT account_id FROM agents WHERE id = $1"#, q.agent_id,)
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
    if !user_is_member(&state.db, user.user_id, agent_account).await? {
        return Err(ApiError::Forbidden);
    }

    // Atomically claim the oldest pending rows for this agent.
    // FOR UPDATE SKIP LOCKED lets concurrent pollers safely pull
    // disjoint batches without blocking each other.
    let mut tx = state.db.begin().await?;
    let claimed = sqlx::query!(
        r#"
        WITH picked AS (
            SELECT id FROM relay_invocations
            WHERE granter_agent_id = $1
              AND status = 'pending'
            ORDER BY created_at ASC
            LIMIT $2
            FOR UPDATE SKIP LOCKED
        )
        UPDATE relay_invocations i
        SET status = 'in_progress',
            claimed_at = now(),
            claimed_by_user_id = $3
        FROM picked
        WHERE i.id = picked.id
        RETURNING i.id
        "#,
        q.agent_id,
        limit,
        user.user_id,
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    if claimed.is_empty() {
        return Ok(Json(vec![]));
    }

    let ids: Vec<Uuid> = claimed.into_iter().map(|r| r.id).collect();
    // Bundle in friendship + grant context. The relay just verified
    // both before queuing this row, so the receiving agent doesn't
    // need to re-check — saves an LLM tool call (or three) per
    // invocation.
    let rows = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at, i.claimed_at,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",

            g.status              as "g_status?",
            g.granter_agent_id    as "g_granter_agent_id?",
            g.grantee_agent_id    as "g_grantee_agent_id?",
            g.capability_id       as "g_capability_id?",
            cap.visibility        as "g_capability_visibility?",
            cap.semantics         as "capability_semantics?",
            g.granted_at          as "g_granted_at?",
            g.expires_at          as "g_expires_at?",

            f.id                  as "f_id?",
            f.status              as "f_status?",
            f.proposer_agent_id   as "f_proposer_agent_id?",
            f.target_agent_id     as "f_target_agent_id?",
            f.proposer_message    as "f_proposer_message?",
            f.response_message    as "f_response_message?",
            f.decided_at          as "f_decided_at?"
        FROM relay_invocations i
        LEFT JOIN agents ga             ON ga.id  = i.granter_agent_id
        LEFT JOIN agents ea             ON ea.id  = i.grantee_agent_id
        LEFT JOIN grants g              ON g.id   = i.grant_id
        -- Source the capability via the invocation's own column rather
        -- than through the grant, so public invokes (grant_id IS NULL)
        -- still carry the correct HITL `semantics` + capability_name on
        -- inbox pull. Trusted invokes always have i.capability_id =
        -- g.capability_id, so this is a no-op for them. (Migration 0022.)
        LEFT JOIN agent_capabilities cap ON cap.id = i.capability_id
        LEFT JOIN LATERAL (
            SELECT *
            FROM friendships f2
            WHERE f2.status = 'accepted'
              AND (
                  (f2.proposer_agent_id = i.granter_agent_id AND f2.target_agent_id = i.grantee_agent_id)
               OR (f2.proposer_agent_id = i.grantee_agent_id AND f2.target_agent_id = i.granter_agent_id)
              )
            ORDER BY f2.decided_at DESC
            LIMIT 1
        ) f ON true
        WHERE i.id = ANY($1)
        ORDER BY i.created_at ASC
        "#,
        &ids,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let grant_context = match (
                    r.grant_id,
                    r.g_status.clone(),
                    r.g_granter_agent_id,
                    r.g_grantee_agent_id,
                    r.g_capability_id,
                    r.g_capability_visibility.clone(),
                    r.g_granted_at,
                ) {
                    (
                        Some(grant_id),
                        Some(status),
                        Some(granter),
                        Some(grantee),
                        Some(capability_id),
                        Some(visibility),
                        Some(granted_at),
                    ) => Some(GrantContext {
                        id: grant_id,
                        status,
                        granter_agent_id: granter,
                        grantee_agent_id: grantee,
                        capability_id,
                        capability_name: r.capability_name.clone(),
                        capability_visibility: visibility,
                        granted_at,
                        expires_at: r.g_expires_at,
                    }),
                    _ => None,
                };
                let friendship_context = match (
                    r.f_id,
                    r.f_status.clone(),
                    r.f_proposer_agent_id,
                    r.f_target_agent_id,
                ) {
                    (Some(id), Some(status), Some(proposer), Some(target)) => {
                        Some(FriendshipContext {
                            id,
                            status,
                            proposer_agent_id: proposer,
                            target_agent_id: target,
                            proposer_message: r.f_proposer_message,
                            response_message: r.f_response_message,
                            decided_at: r.f_decided_at,
                        })
                    }
                    _ => None,
                };

                InvocationDto {
                    id: r.id,
                    grant_id: r.grant_id,
                    granter_agent_id: r.granter_agent_id,
                    granter_display_name: r.granter_display_name,
                    grantee_agent_id: r.grantee_agent_id,
                    grantee_display_name: r.grantee_display_name,
                    capability_id: r.capability_id,
                    capability_name: r.capability_name,
                    semantics: r.capability_semantics,
                    status: r.status,
                    elapsed_ms: r.elapsed_ms,
                    error_message: r.error_message,
                    input_preview: r.input_preview,
                    output_preview: r.output_preview,
                    created_at: r.created_at,
                    claimed_at: r.claimed_at,
                    i_served: true,
                    i_invoked: false,
                    tier: if r.grant_id.is_some() {
                        "friend".into()
                    } else {
                        "public".into()
                    },
                    friendship_context,
                    grant_context,
                }
            })
            .collect(),
    ))
}

// ─── POST /v1/invocations/{id}/result ────────────────────
//
// Returns `Response` (rather than `ApiResult<Json<InvocationDto>>`) so
// the HITL policy gate can emit a 409 with a custom error code —
// `chk.policy.requires_human_confirmation` — that the shared
// `ApiError::Conflict` envelope (code = `"conflict"`) can't express.
// The success + happy-path errors still flow through `ApiError` and its
// `IntoResponse`.
pub async fn report_result(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ResultRequest>,
) -> Result<Response, ApiError> {
    if !matches!(req.status.as_str(), "succeeded" | "failed") {
        return Err(ApiError::InvalidRequest(
            "status must be 'succeeded' or 'failed'".into(),
        ));
    }

    // Join `agent_capabilities` to pick up the HITL `semantics` column
    // alongside the invocation's status + granter account. The
    // capability may be null on legacy rows that pre-date the
    // capability_id column — in that case `semantics` comes back as
    // None and the gate is a no-op (matches the migration's
    // backwards-compatible default).
    let row = sqlx::query!(
        r#"
        SELECT i.status, i.created_at, i.claimed_at,
               ga.account_id as "granter_account_id?",
               c.semantics   as "capability_semantics?"
        FROM relay_invocations i
        LEFT JOIN agents              ga ON ga.id = i.granter_agent_id
        LEFT JOIN agent_capabilities  c  ON c.id  = i.capability_id
        WHERE i.id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.status != "in_progress" {
        return Err(ApiError::Conflict(format!(
            "invocation is {}; only in_progress can be completed",
            row.status
        )));
    }

    // Auth: caller must be a member of the granter's account. The agent
    // could be deleted in the meantime; if so, granter_account_id is
    // null and no one can report a result — surface as forbidden.
    let granter_account = row.granter_account_id.ok_or(ApiError::Forbidden)?;
    if !user_is_member(&state.db, user.user_id, granter_account).await? {
        return Err(ApiError::Forbidden);
    }

    // HITL gate (issue #69 PR 2): if the capability is `human_in_loop`,
    // refuse the result unless `confirmed_by_human: true`. Returning a
    // custom 409 body keeps SDK callers' error-classification logic in
    // sync with the documented contract on the migration:
    //
    //   POST /v1/invocations/{id}/result
    //   → 409 { error: { code: "chk.policy.requires_human_confirmation", … } }
    if row.capability_semantics.as_deref() == Some("human_in_loop")
        && !req.confirmed_by_human.unwrap_or(false)
    {
        let body = serde_json::json!({
            "error": {
                "code": "chk.policy.requires_human_confirmation",
                "message": "this capability requires explicit human confirmation; set `confirmed_by_human: true` if a human approved the response",
            }
        });
        return Ok((StatusCode::CONFLICT, Json(body)).into_response());
    }

    // Wall time from enqueue to result.
    let elapsed_ms = (Utc::now() - row.created_at)
        .num_milliseconds()
        .clamp(0, i32::MAX as i64) as i32;
    let output_preview = req.output.as_ref().map(truncate_for_audit);

    sqlx::query!(
        r#"
        UPDATE relay_invocations
        SET status = $2,
            elapsed_ms = $3,
            error_message = $4,
            output_preview = $5
        WHERE id = $1 AND status = 'in_progress'
        "#,
        id,
        req.status,
        elapsed_ms,
        req.error.as_deref(),
        output_preview.unwrap_or(Value::Null),
    )
    .execute(&state.db)
    .await?;

    let r = fetch_one(&state.db, user.user_id, id).await?;
    Ok(Json(r).into_response())
}

// ─── GET /v1/invocations ─────────────────────────────────
pub async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<InvocationDto>>> {
    let direction = q.direction.as_deref().unwrap_or("all");
    if !matches!(direction, "all" | "outbound" | "inbound") {
        return Err(ApiError::InvalidRequest(
            "direction must be all|outbound|inbound".into(),
        ));
    }
    if let Some(s) = q.status.as_deref() {
        if !matches!(
            s,
            "pending" | "in_progress" | "rejected" | "succeeded" | "failed" | "timeout"
        ) {
            return Err(ApiError::InvalidRequest("invalid status".into()));
        }
    }

    // outbound = I'm the grantee (caller) for the row; inbound = I'm
    // the granter (server). The booleans below feed into the SQL
    // WHERE clause: `want_grantee_is_me` flips on for outbound + all,
    // `want_granter_is_me` for inbound + all. Renamed from the older
    // (and confusingly inverted) `want_outbound`/`want_inbound`.
    let want_grantee_is_me = matches!(direction, "all" | "outbound");
    let want_granter_is_me = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at, i.claimed_at, i.trust_snapshot,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",
            cap.semantics   as "capability_semantics?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ga.account_id
            ) as "i_served?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ea.account_id
            ) as "i_invoked?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        LEFT JOIN agent_capabilities cap ON cap.id = i.capability_id
        WHERE
            (
                -- $2 = want_grantee_is_me (I sent this call OUT)
                ($2::boolean AND ea.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                ))
                -- $3 = want_granter_is_me (call came IN, I served it)
             OR ($3::boolean AND ga.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                ))
            )
            AND ($4::uuid IS NULL OR i.granter_agent_id = $4 OR i.grantee_agent_id = $4)
            AND ($5::text IS NULL OR i.status = $5)
        ORDER BY i.created_at DESC
        LIMIT 200
        "#,
        user.user_id,
        want_grantee_is_me,
        want_granter_is_me,
        q.agent_id,
        q.status,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let (friendship_context, grant_context) =
                    contexts_from_snapshot(r.trust_snapshot.as_ref());
                InvocationDto {
                    id: r.id,
                    grant_id: r.grant_id,
                    granter_agent_id: r.granter_agent_id,
                    granter_display_name: r.granter_display_name,
                    grantee_agent_id: r.grantee_agent_id,
                    grantee_display_name: r.grantee_display_name,
                    capability_id: r.capability_id,
                    capability_name: r.capability_name,
                    semantics: r.capability_semantics,
                    status: r.status,
                    elapsed_ms: r.elapsed_ms,
                    error_message: r.error_message,
                    input_preview: r.input_preview,
                    output_preview: r.output_preview,
                    created_at: r.created_at,
                    claimed_at: r.claimed_at,
                    i_served: r.i_served.unwrap_or(false),
                    i_invoked: r.i_invoked.unwrap_or(false),
                    tier: if r.grant_id.is_some() {
                        "friend".into()
                    } else {
                        "public".into()
                    },
                    friendship_context,
                    grant_context,
                }
            })
            .collect(),
    ))
}

// ─── GET /v1/invocations/{id} ────────────────────────────
pub async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InvocationDto>> {
    Ok(Json(fetch_one(&state.db, user.user_id, id).await?))
}

async fn fetch_one(db: &PgPool, user_id: Uuid, id: Uuid) -> Result<InvocationDto, ApiError> {
    let r = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at, i.claimed_at, i.trust_snapshot,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",
            cap.semantics   as "capability_semantics?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ga.account_id
            ) as "i_served?",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.user_id = $1 AND m.account_id = ea.account_id
            ) as "i_invoked?"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        LEFT JOIN agent_capabilities cap ON cap.id = i.capability_id
        WHERE i.id = $2
        "#,
        user_id,
        id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let i_served = r.i_served.unwrap_or(false);
    let i_invoked = r.i_invoked.unwrap_or(false);
    if !i_served && !i_invoked {
        return Err(ApiError::NotFound);
    }

    let (friendship_context, grant_context) = contexts_from_snapshot(r.trust_snapshot.as_ref());

    Ok(InvocationDto {
        id: r.id,
        grant_id: r.grant_id,
        granter_agent_id: r.granter_agent_id,
        granter_display_name: r.granter_display_name,
        grantee_agent_id: r.grantee_agent_id,
        grantee_display_name: r.grantee_display_name,
        capability_id: r.capability_id,
        capability_name: r.capability_name,
        semantics: r.capability_semantics,
        status: r.status,
        elapsed_ms: r.elapsed_ms,
        error_message: r.error_message,
        input_preview: r.input_preview,
        output_preview: r.output_preview,
        created_at: r.created_at,
        claimed_at: r.claimed_at,
        i_served,
        i_invoked,
        tier: if r.grant_id.is_some() {
            "friend".into()
        } else {
            "public".into()
        },
        friendship_context,
        grant_context,
    })
}

/// Deserialize the `trust_snapshot` JSONB blob (migration 0024) back
/// into the typed contexts. The snapshot's shape is
/// `{"friendship": {...}, "grant": {...}}` with the same fields as
/// the live FriendshipContext + GrantContext. Used by audit-log
/// endpoints (list, get) so they return state-as-of-queue-time
/// instead of state-now or null.
///
/// On rows whose snapshot is NULL (public-invoke + legacy pre-0024
/// rows), both come back None — same as before this change. We
/// silently swallow malformed snapshots; a future schema migration
/// should be an additive ALTER rather than a destructive one.
fn contexts_from_snapshot(
    snapshot: Option<&Value>,
) -> (Option<FriendshipContext>, Option<GrantContext>) {
    let Some(snap) = snapshot else {
        return (None, None);
    };
    let f = snap
        .get("friendship")
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::from_value::<FriendshipContext>(v.clone()).ok());
    let g = snap
        .get("grant")
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::from_value::<GrantContext>(v.clone()).ok());
    (f, g)
}

#[cfg(test)]
mod legacy_v01_contract_tests {
    //! D6 regression gate: the v0.1.0 SDK contract for the legacy
    //! pull-mode endpoints must continue to work post-A2A migration,
    //! since v0.1.0 of `chakramcp` (Python) and `@chakramcp/sdk` (TS)
    //! are already published and depend on these exact endpoint
    //! shapes:
    //!
    //!   POST /v1/invoke                 — caller submits work
    //!   GET  /v1/invocations/{id}       — caller polls for terminal
    //!   GET  /v1/inbox?agent_id=…       — granter polls for parked
    //!   POST /v1/invocations/{id}/result — granter reports outcome
    //!
    //! The scheduler-demo (`examples/scheduler-demo/`) is a
    //! published-on-the-website v0.1.0 SDK demo that uses exactly
    //! this flow. If this test fails, the demo fails. D5 doesn't
    //! touch any of these handlers — but the schema migrations
    //! from D1 *did* touch the agents/accounts tables, so we
    //! verify the contract holds end-to-end.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sha2::{Digest, Sha256};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    pub(super) fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            // Off — legacy endpoints don't depend on it. Keeping it
            // false also proves the v0.1.0 contract works for
            // self-hosters who haven't enabled the new A2A surfaces.
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    /// Seed the same shape as `examples/scheduler-demo/setup.py`:
    /// two accounts, one agent each, friendship, grant. Returns the
    /// caller's ck_ token, both agent ids, and the grant id.
    pub(super) struct DemoFixture {
        pub caller_token: String,
        pub caller_user_id: Uuid,
        pub granter_agent_id: Uuid,
        pub grantee_agent_id: Uuid,
        pub grant_id: Uuid,
        pub capability_id: Uuid,
    }

    pub(super) async fn seed_demo(pool: &PgPool) -> DemoFixture {
        let alice_user = Uuid::now_v7(); // granter / inbox.serve side
        let bob_user = Uuid::now_v7(); // grantee / invoke side
        for (uid, name) in [(alice_user, "Alice"), (bob_user, "Bob")] {
            sqlx::query!(
                r#"INSERT INTO users (id, email, display_name, password_hash)
                   VALUES ($1, $2, $3, 'x')"#,
                uid,
                format!("{name}-{uid}@t.local"),
                name,
            )
            .execute(pool)
            .await
            .unwrap();
        }

        // Bob (the caller) gets an API key — same shape v0.1.0 SDK uses.
        let bob_ck = format!("ck_demo_{bob_user}");
        let mut h = Sha256::new();
        h.update(bob_ck.as_bytes());
        let kh = hex::encode(h.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'demo', 'ck_demo')"#,
            Uuid::now_v7(),
            bob_user,
            kh,
        )
        .execute(pool)
        .await
        .unwrap();

        let alice_account = Uuid::now_v7();
        let bob_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Alice', 'individual', $3),
                      ($4, $5, 'Bob',   'individual', $6)"#,
            alice_account,
            format!("alice-{alice_account}"),
            alice_user,
            bob_account,
            format!("bob-{bob_account}"),
            bob_user,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner'),
                      ($4, $5, $6, 'owner')"#,
            Uuid::now_v7(),
            alice_account,
            alice_user,
            Uuid::now_v7(),
            bob_account,
            bob_user,
        )
        .execute(pool)
        .await
        .unwrap();

        // Two agents: alice-scheduler (granter) + bob-caller (grantee).
        let alice_agent = Uuid::now_v7();
        let bob_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'alice-scheduler', 'Alice Scheduler', 'network'),
                      ($3, $4, 'bob-caller',      'Bob Caller',      'network')"#,
            alice_agent,
            alice_account,
            bob_agent,
            bob_account,
        )
        .execute(pool)
        .await
        .unwrap();

        // The propose_slots capability on alice-scheduler.
        let cap = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, 'propose_slots', 'Propose meeting slots.',
                       '{"type":"object"}'::jsonb,
                       '{"type":"object"}'::jsonb,
                       'network')"#,
            cap,
            alice_agent,
        )
        .execute(pool)
        .await
        .unwrap();

        // Friendship + Grant.
        sqlx::query!(
            r#"INSERT INTO friendships
                  (id, proposer_agent_id, target_agent_id, status, decided_at)
               VALUES ($1, $2, $3, 'accepted', now())"#,
            Uuid::now_v7(),
            alice_agent,
            bob_agent,
        )
        .execute(pool)
        .await
        .unwrap();
        let grant = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO grants
                  (id, granter_agent_id, grantee_agent_id, capability_id, status)
               VALUES ($1, $2, $3, $4, 'active')"#,
            grant,
            alice_agent,
            bob_agent,
            cap,
        )
        .execute(pool)
        .await
        .unwrap();

        DemoFixture {
            caller_token: bob_ck,
            caller_user_id: bob_user,
            granter_agent_id: alice_agent,
            grantee_agent_id: bob_agent,
            grant_id: grant,
            capability_id: cap,
        }
    }

    /// Full flow mirrors what `chakramcp.AsyncChakraMCP.invoke_and_wait`
    /// + `chakramcp.AsyncChakraMCP.inbox.serve` do at the wire level.
    /// If this test passes, the scheduler-demo passes.
    #[sqlx::test(migrations = "../migrations")]
    async fn v01_pull_mode_full_lifecycle(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        // ── Bob: POST /v1/invoke ────────────────────────────
        let invoke_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/invoke")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "grant_id": f.grant_id,
                            "grantee_agent_id": f.grantee_agent_id,
                            "input": {"duration_min": 30, "within_days": 7}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invoke_res.status(), StatusCode::ACCEPTED);
        let invoke_body: serde_json::Value =
            serde_json::from_slice(&invoke_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(invoke_body["status"], "pending");
        let invocation_id: Uuid = invoke_body["invocation_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap();

        // ── Bob: GET /v1/invocations/{id} (still pending) ───
        let poll_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/invocations/{invocation_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(poll_res.status(), StatusCode::OK);
        let poll_body: serde_json::Value =
            serde_json::from_slice(&poll_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(poll_body["status"], "pending");

        // ── Alice: GET /v1/inbox?agent_id=<alice-scheduler> ─
        // Need a token for Alice. Mint one quickly.
        let alice_user_id = sqlx::query_scalar!(
            "SELECT owner_user_id FROM accounts a JOIN agents g ON g.account_id=a.id WHERE g.id=$1",
            f.granter_agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
        let alice_ck = format!("ck_demo_alice_{alice_user_id}");
        let mut h = Sha256::new();
        h.update(alice_ck.as_bytes());
        let kh = hex::encode(h.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'demo', 'ck_demo')"#,
            Uuid::now_v7(),
            alice_user_id,
            kh,
        )
        .execute(&pool)
        .await
        .unwrap();

        let inbox_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/inbox?agent_id={}", f.granter_agent_id))
                    .header(header::AUTHORIZATION, format!("Bearer {alice_ck}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(inbox_res.status(), StatusCode::OK);
        let inbox_body: serde_json::Value =
            serde_json::from_slice(&inbox_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        // The legacy contract returns an array of invocations.
        let items = inbox_body
            .get("invocations")
            .or_else(|| inbox_body.as_array().map(|_| &inbox_body))
            .and_then(|v| v.as_array())
            .expect("inbox response should contain an invocations array");
        assert!(
            items
                .iter()
                .any(|inv| inv["id"] == invocation_id.to_string()),
            "Alice's inbox should include the parked invocation"
        );

        // ── Alice: POST /v1/invocations/{id}/result ─────────
        let result_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/invocations/{invocation_id}/result"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_ck}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "status": "succeeded",
                            "output": { "slots": ["2026-05-04T10:00:00Z"] }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            result_res.status().is_success(),
            "result post should succeed; got {}",
            result_res.status()
        );

        // ── Bob: GET /v1/invocations/{id} (now succeeded) ───
        let poll2 = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/invocations/{invocation_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let poll2_body: serde_json::Value =
            serde_json::from_slice(&poll2.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(poll2_body["status"], "succeeded");
        assert_eq!(
            poll2_body["output_preview"]["slots"][0],
            "2026-05-04T10:00:00Z"
        );

        // Defensive use of unused values.
        let _ = (
            f.caller_user_id,
            f.capability_id,
            f.grantee_agent_id,
            invocation_id,
        );
    }

    /// API-key usage attribution: when the caller invokes through
    /// `POST /v1/invoke` with a `ck_` Bearer, the resulting
    /// `relay_invocations` row's `api_key_id` matches that key's row in
    /// `api_keys`. This is what the per-key dashboard at
    /// `/v1/api-keys/{id}/usage` joins on; if it ever drifts to NULL
    /// the charts go to zero (the bug this regression test guards).
    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_records_api_key_id(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        // The api_keys.id corresponding to the ck_ token from seed_demo.
        let mut h = Sha256::new();
        h.update(f.caller_token.as_bytes());
        let kh = hex::encode(h.finalize());
        let expected_key_id: Uuid =
            sqlx::query_scalar!(r#"SELECT id FROM api_keys WHERE key_hash = $1"#, kh)
                .fetch_one(&pool)
                .await
                .unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/invoke")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "grant_id": f.grant_id,
                            "grantee_agent_id": f.grantee_agent_id,
                            "input": {"k": "v"}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let invocation_id: Uuid = body["invocation_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap();

        let row_key_id: Option<Uuid> = sqlx::query_scalar!(
            r#"SELECT api_key_id FROM relay_invocations WHERE id = $1"#,
            invocation_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row_key_id,
            Some(expected_key_id),
            "ck_ caller should leave api_key_id on the invocation row",
        );
    }

    /// Negative case: when the caller is NOT on a ck_ token (i.e. the
    /// JWT path through `AuthUser`), `record_terminal` is invoked with
    /// `api_key_id = None` and the row's column stays NULL. The
    /// per-key dashboard filters by api_key_id, so JWT traffic stays
    /// out of those totals.
    ///
    /// We exercise `record_terminal` directly instead of going through
    /// `/v1/invoke` because the relay test suite's JWT path is blocked
    /// on a pre-existing test infra bug (jsonwebtoken 10.x picked up in
    /// PR #36 panics at decode time inside the test runner unless a
    /// rustls CryptoProvider is installed — see the same panic across
    /// all `handlers::agents::*` tests on main). The column-write logic
    /// is the same, and a future test infra fix can replace this with
    /// an end-to-end JWT call.
    #[sqlx::test(migrations = "../migrations")]
    async fn record_terminal_leaves_api_key_id_null_for_jwt_callers(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let id = super::record_terminal(
            &pool,
            Some(f.grant_id),
            Some(f.granter_agent_id),
            Some(f.grantee_agent_id),
            Some(f.capability_id),
            "propose_slots",
            f.caller_user_id,
            "rejected",
            0,
            Some("simulated JWT-path rejection"),
            Some(&serde_json::json!({"k": "v"})),
            None, // ← the JWT path: api_key_id is None
            None, // minted_jti — also None for this synthetic call
        )
        .await
        .unwrap();

        let row_key_id: Option<Uuid> = sqlx::query_scalar!(
            r#"SELECT api_key_id FROM relay_invocations WHERE id = $1"#,
            id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            row_key_id.is_none(),
            "JWT-authed callers must leave api_key_id NULL; got {row_key_id:?}",
        );
    }
}

// ─── HITL gate tests (issue #69 PR 2) ─────────────────────
#[cfg(test)]
mod hitl_gate_tests {
    //! Coverage for the new `confirmed_by_human` gate in
    //! `report_result`. Three cases:
    //!
    //! 1. `human_in_loop` + flag true → 200, row goes `succeeded`.
    //! 2. `human_in_loop` + flag missing → 409 with the documented
    //!    `chk.policy.requires_human_confirmation` envelope; row stays
    //!    `in_progress`.
    //! 3. `autonomous` + no flag → 200, regression check that the
    //!    gate is a strict no-op for non-HITL capabilities (the bulk
    //!    of existing capabilities).
    //!
    //! All three reuse `seed_demo` from the legacy module and then
    //! drive the invocation through `/v1/invoke` → claim via inbox →
    //! report result, mirroring the v0.1.0 SDK lifecycle exercised by
    //! `v01_pull_mode_full_lifecycle`.
    use super::legacy_v01_contract_tests::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use sha2::{Digest, Sha256};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Drive a fresh invocation up to `in_progress` and return
    /// `(invocation_id, alice_token)`. Optionally flips the capability
    /// to `human_in_loop` before invoking — when `hitl` is true, the
    /// gate in `report_result` is in scope for any subsequent
    /// `/result` post.
    async fn invoke_and_claim(
        pool: &PgPool,
        app: &axum::Router,
        f: &DemoFixture,
        hitl: bool,
    ) -> (Uuid, String) {
        if hitl {
            sqlx::query!(
                "UPDATE agent_capabilities SET semantics = 'human_in_loop' WHERE id = $1",
                f.capability_id,
            )
            .execute(pool)
            .await
            .unwrap();
        }

        // Bob: POST /v1/invoke
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/invoke")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "grant_id": f.grant_id,
                            "grantee_agent_id": f.grantee_agent_id,
                            "input": {"k": "v"}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let invocation_id: Uuid = body["invocation_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap();

        // Mint Alice (the granter) a ck_ token + claim the row via inbox.
        let alice_user_id = sqlx::query_scalar!(
            "SELECT owner_user_id FROM accounts a JOIN agents g ON g.account_id=a.id WHERE g.id=$1",
            f.granter_agent_id,
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .unwrap();
        let alice_ck = format!("ck_demo_alice_{alice_user_id}");
        let mut h = Sha256::new();
        h.update(alice_ck.as_bytes());
        let kh = hex::encode(h.finalize());
        sqlx::query!(
            r#"INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix)
               VALUES ($1, $2, $3, 'demo', 'ck_demo')"#,
            Uuid::now_v7(),
            alice_user_id,
            kh,
        )
        .execute(pool)
        .await
        .unwrap();

        // Pull from the inbox — atomically flips the row to in_progress
        // so `report_result` accepts it.
        let inbox_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/inbox?agent_id={}", f.granter_agent_id))
                    .header(header::AUTHORIZATION, format!("Bearer {alice_ck}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(inbox_res.status(), StatusCode::OK);

        (invocation_id, alice_ck)
    }

    async fn post_result(
        app: &axum::Router,
        invocation_id: Uuid,
        alice_ck: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/invocations/{invocation_id}/result"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_ck}"))
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        // Some success bodies parse as JSON; on 409 the body is JSON
        // too. Default to Null when empty.
        let v: serde_json::Value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, v)
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn respond_with_confirmed_by_human_true_on_hitl_succeeds(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let (invocation_id, alice_ck) = invoke_and_claim(&pool, &app, &f, true).await;

        let (status, _body) = post_result(
            &app,
            invocation_id,
            &alice_ck,
            serde_json::json!({
                "status": "succeeded",
                "output": { "reply": "ok" },
                "confirmed_by_human": true,
            }),
        )
        .await;
        assert!(
            status.is_success(),
            "HITL + confirmed_by_human=true should succeed; got {status}"
        );

        let row_status: String = sqlx::query_scalar!(
            "SELECT status FROM relay_invocations WHERE id = $1",
            invocation_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row_status, "succeeded");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn respond_without_confirmed_by_human_on_hitl_returns_409(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let (invocation_id, alice_ck) = invoke_and_claim(&pool, &app, &f, true).await;

        // No `confirmed_by_human` field at all — most common case for
        // an SDK that hasn't opted in.
        let (status, body) = post_result(
            &app,
            invocation_id,
            &alice_ck,
            serde_json::json!({
                "status": "succeeded",
                "output": { "reply": "ok" },
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body["error"]["code"],
            "chk.policy.requires_human_confirmation",
        );

        // The row must stay in_progress so a follow-up call with the
        // flag set can still succeed.
        let row_status: String = sqlx::query_scalar!(
            "SELECT status FROM relay_invocations WHERE id = $1",
            invocation_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row_status, "in_progress");

        // Explicit false should behave identically to missing.
        let (status2, body2) = post_result(
            &app,
            invocation_id,
            &alice_ck,
            serde_json::json!({
                "status": "succeeded",
                "output": { "reply": "ok" },
                "confirmed_by_human": false,
            }),
        )
        .await;
        assert_eq!(status2, StatusCode::CONFLICT);
        assert_eq!(
            body2["error"]["code"],
            "chk.policy.requires_human_confirmation",
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn respond_without_confirmed_by_human_on_autonomous_still_succeeds(pool: PgPool) {
        // Regression: the gate must NOT fire on the default
        // `autonomous` semantics — that's every pre-PR-1 capability
        // and the bulk of post-PR-1 ones.
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let (invocation_id, alice_ck) = invoke_and_claim(&pool, &app, &f, false).await;

        let (status, _body) = post_result(
            &app,
            invocation_id,
            &alice_ck,
            serde_json::json!({
                "status": "succeeded",
                "output": { "reply": "ok" },
            }),
        )
        .await;
        assert!(
            status.is_success(),
            "autonomous capability must accept results without the flag; got {status}"
        );

        let row_status: String = sqlx::query_scalar!(
            "SELECT status FROM relay_invocations WHERE id = $1",
            invocation_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row_status, "succeeded");
    }
}

#[cfg(test)]
mod public_invoke_tests {
    //! Coverage for the public-invoke path introduced in migration 0022.
    //! See `docs/superpowers/specs/2026-05-23-public-invokable-capabilities-design.md`.

    use super::legacy_v01_contract_tests::config;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::jwt::{encode_jwt, UserClaims};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Two users + their personal accounts; the granter has a network
    /// agent with a `public_invoke=true` capability, plus a regular
    /// (friend-only) capability for the negative case. The invoker
    /// (Bob) has his own agent, private, no friendship with the
    /// granter. Returns the ids needed by each test.
    struct PubFixture {
        granter_user: Uuid,
        granter_agent: Uuid,
        invoker_user: Uuid,
        invoker_account: Uuid,
        invoker_agent: Uuid,
        invoker_token: String,
        public_cap: Uuid,
        friend_only_cap: Uuid,
    }

    async fn seed_public(pool: &PgPool, quota: i32, hitl: bool) -> PubFixture {
        let granter_user = Uuid::now_v7();
        let invoker_user = Uuid::now_v7();
        for (uid, name) in [(granter_user, "Granter"), (invoker_user, "Invoker")] {
            sqlx::query!(
                r#"INSERT INTO users (id, email, display_name, password_hash)
                   VALUES ($1, $2, $3, 'x')"#,
                uid,
                format!("{name}-{uid}@t.local"),
                name,
            )
            .execute(pool)
            .await
            .unwrap();
        }

        let granter_account = Uuid::now_v7();
        let invoker_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Granter', 'individual', $3),
                      ($4, $5, 'Invoker', 'individual', $6)"#,
            granter_account,
            format!("granter-{granter_account}"),
            granter_user,
            invoker_account,
            format!("invoker-{invoker_account}"),
            invoker_user,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner'),
                      ($4, $5, $6, 'owner')"#,
            Uuid::now_v7(),
            granter_account,
            granter_user,
            Uuid::now_v7(),
            invoker_account,
            invoker_user,
        )
        .execute(pool)
        .await
        .unwrap();

        let granter_agent = Uuid::now_v7();
        let invoker_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'granter-svc', 'Granter Svc', 'network'),
                      ($3, $4, 'invoker-cli', 'Invoker CLI', 'private')"#,
            granter_agent,
            granter_account,
            invoker_agent,
            invoker_account,
        )
        .execute(pool)
        .await
        .unwrap();

        let semantics = if hitl { "human_in_loop" } else { "autonomous" };
        let public_cap = Uuid::now_v7();
        let friend_only_cap = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema,
                   visibility, semantics, public_invoke, public_monthly_quota_per_agent)
               VALUES ($1, $2, 'public_op', '', '{}'::jsonb, '{}'::jsonb,
                       'network', $3, true, $4),
                      ($5, $6, 'friend_op', '', '{}'::jsonb, '{}'::jsonb,
                       'network', 'autonomous', false, NULL)"#,
            public_cap,
            granter_agent,
            semantics,
            quota,
            friend_only_cap,
            granter_agent,
        )
        .execute(pool)
        .await
        .unwrap();

        // The invoker authenticates with a JWT. Same `test-secret` the
        // legacy fixture uses (see `config()` in legacy_v01_contract_tests).
        let token = encode_jwt(
            &UserClaims::new(invoker_user, format!("{invoker_user}@t.local"), false, 1),
            "test-secret",
        )
        .unwrap();

        PubFixture {
            granter_user,
            granter_agent,
            invoker_user,
            invoker_account,
            invoker_agent,
            invoker_token: token,
            public_cap,
            friend_only_cap,
        }
    }

    fn invoke_req(token: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/invoke")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    /// Happy path. Non-friend invokes a public_invoke=true capability →
    /// 202, row written with grant_id=NULL + correct grantee/granter +
    /// status=pending.
    #[sqlx::test(migrations = "../migrations")]
    async fn public_invoke_enqueues_with_null_grant(pool: PgPool) {
        let f = seed_public(&pool, 5, false).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.invoker_agent,
                    "input": {"hi": 1}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED, "expected 202");
        let body: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["status"], "pending");
        let id: Uuid = body["invocation_id"].as_str().unwrap().parse().unwrap();

        let row = sqlx::query!(
            r#"SELECT grant_id, granter_agent_id, grantee_agent_id, capability_id, status
               FROM relay_invocations WHERE id = $1"#,
            id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            row.grant_id.is_none(),
            "public invoke must write grant_id = NULL"
        );
        assert_eq!(row.granter_agent_id, Some(f.granter_agent));
        assert_eq!(row.grantee_agent_id, Some(f.invoker_agent));
        assert_eq!(row.capability_id, Some(f.public_cap));
        assert_eq!(row.status, "pending");
        // Silence warnings about unused fields in the fixture.
        let _ = (f.granter_user, f.invoker_user);
    }

    /// Friend-only capability via the public path → 404. Don't leak
    /// whether a non-public capability exists.
    #[sqlx::test(migrations = "../migrations")]
    async fn friend_only_capability_via_public_path_is_404(pool: PgPool) {
        let f = seed_public(&pool, 5, false).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "capability_id": f.friend_only_cap,
                    "grantee_agent_id": f.invoker_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// Per-invoker monthly quota: 2 enqueues succeed, third → 429 with
    /// structured body. Consumed on enqueue (status stays `pending` —
    /// no downstream completion needed to consume quota).
    #[sqlx::test(migrations = "../migrations")]
    async fn monthly_quota_enforced(pool: PgPool) {
        let f = seed_public(&pool, 2, false).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        for _ in 0..2 {
            let res = app
                .clone()
                .oneshot(invoke_req(
                    &f.invoker_token,
                    serde_json::json!({
                        "capability_id": f.public_cap,
                        "grantee_agent_id": f.invoker_agent,
                        "input": {}
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::ACCEPTED);
        }
        // Third → 429.
        let res = app
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.invoker_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        let body: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["error"]["code"], "monthly_quota_exhausted");
        assert_eq!(body["quota"], 2);
        assert!(body["resets_at"].is_string(), "resets_at must be present");
    }

    /// Quota counts only the current calendar month. Backdate two rows
    /// to the previous month; the third "live" invoke should still
    /// succeed (the prior-month rows don't consume this month's budget).
    #[sqlx::test(migrations = "../migrations")]
    async fn prior_month_invocations_dont_count(pool: PgPool) {
        let f = seed_public(&pool, 2, false).await;
        // Two rows backdated 35 days = previous calendar month.
        for _ in 0..2 {
            sqlx::query!(
                r#"INSERT INTO relay_invocations
                       (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
                        capability_name, invoked_by_user_id, status, created_at)
                   VALUES ($1, NULL, $2, $3, $4, 'public_op', $5, 'succeeded',
                           now() - interval '35 days')"#,
                Uuid::now_v7(),
                f.granter_agent,
                f.invoker_agent,
                f.public_cap,
                f.invoker_user,
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.invoker_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::ACCEPTED,
            "prior-month rows must not count toward this month's quota"
        );
    }

    /// Validation: both grant_id + capability_id → 400; neither → 400.
    #[sqlx::test(migrations = "../migrations")]
    async fn validation_400s(pool: PgPool) {
        let f = seed_public(&pool, 5, false).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        // Both — bogus grant id is fine, the validation fires before the lookup.
        let res = app
            .clone()
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "grant_id": Uuid::now_v7(),
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.invoker_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "both ids → 400");
        // Neither.
        let res = app
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "grantee_agent_id": f.invoker_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "neither id → 400");
    }

    /// Self-invoke (`grantee == granter`) → 400. Calling your own
    /// agent's public capability is pointless and would pollute quota
    /// + future ratings with self-traffic.
    #[sqlx::test(migrations = "../migrations")]
    async fn self_invoke_400(pool: PgPool) {
        let f = seed_public(&pool, 5, false).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        // Invoker tries to invoke the granter's cap "as" the granter agent —
        // which they don't own. To exercise self-invoke we need the invoker
        // to be a member of the granter's account. Easier: register a JWT
        // for the granter user, then attempt to invoke as granter_agent.
        let granter_token = encode_jwt(
            &UserClaims::new(
                f.granter_user,
                format!("{}@t.local", f.granter_user),
                false,
                1,
            ),
            "test-secret",
        )
        .unwrap();
        let res = app
            .oneshot(invoke_req(
                &granter_token,
                serde_json::json!({
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.granter_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// Invoking as an agent the caller does not own → 403. The caller
    /// authenticates as Invoker (the user) but tries to claim it is
    /// invoking as the granter's agent (which the invoker does not own).
    #[sqlx::test(migrations = "../migrations")]
    async fn invoke_as_not_mine_403(pool: PgPool) {
        let f = seed_public(&pool, 5, false).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.granter_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        // suppress unused-field lint
        let _ = f.invoker_account;
    }

    /// Inbox-pull for a public invocation: friendship_context +
    /// grant_context are absent (no trust trail), AND the HITL
    /// `semantics` is sourced from `i.capability_id` (the fix in this
    /// PR), so a `human_in_loop` public capability surfaces its real
    /// classification rather than being silently degraded to autonomous.
    #[sqlx::test(migrations = "../migrations")]
    async fn inbox_pull_carries_correct_semantics_and_null_contexts(pool: PgPool) {
        let f = seed_public(&pool, 5, true).await; // hitl=true
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        // Enqueue a public invocation.
        let res = app
            .clone()
            .oneshot(invoke_req(
                &f.invoker_token,
                serde_json::json!({
                    "capability_id": f.public_cap,
                    "grantee_agent_id": f.invoker_agent,
                    "input": {}
                }),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        // Granter pulls from inbox.
        let granter_token = encode_jwt(
            &UserClaims::new(
                f.granter_user,
                format!("{}@t.local", f.granter_user),
                false,
                1,
            ),
            "test-secret",
        )
        .unwrap();
        let inbox_res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/inbox?agent_id={}", f.granter_agent))
                    .header(header::AUTHORIZATION, format!("Bearer {granter_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(inbox_res.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&inbox_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let arr = body.as_array().expect("array");
        assert_eq!(arr.len(), 1, "exactly one row should be pulled");
        let row = &arr[0];
        // HITL signal must survive the grantless path.
        assert_eq!(
            row["semantics"], "human_in_loop",
            "public invoke must carry the cap's real semantics — null would silently degrade to autonomous"
        );
        // No grant_id on a public invoke.
        assert!(row["grant_id"].is_null());
        // tier field stamped by the relay so handlers don't have to
        // infer "null grant_id = public" themselves.
        assert_eq!(
            row["tier"], "public",
            "public invoke must stamp tier='public'"
        );
        // Trust contexts must be absent (serde_skip → key not present).
        assert!(
            row.get("friendship_context").is_none() || row["friendship_context"].is_null(),
            "no friendship context on public invoke"
        );
        assert!(
            row.get("grant_context").is_none() || row["grant_context"].is_null(),
            "no grant context on public invoke"
        );
    }
}

#[cfg(test)]
mod tier_field_tests {
    //! Edge 1 from the trust-context audit: the `tier` field on
    //! `InvocationDto` must be `"friend"` for grant-path invocations
    //! and `"public"` for public-invoke calls, and that signal must
    //! survive across all three read endpoints (inbox.pull, list, get)
    //! so handlers + audit-log consumers can use it uniformly.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::jwt::{encode_jwt, UserClaims};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;

    use super::legacy_v01_contract_tests::{config, seed_demo};

    #[sqlx::test(migrations = "../migrations")]
    async fn grant_path_invocation_carries_tier_friend(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        // Bob (caller) → POST /v1/invoke against Alice's grant.
        let invoke_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/invoke")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "grant_id": f.grant_id,
                            "grantee_agent_id": f.grantee_agent_id,
                            "input": {"duration_min": 30}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invoke_res.status(), StatusCode::ACCEPTED);
        let invoke_body: Value =
            serde_json::from_slice(&invoke_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let invocation_id = invoke_body["invocation_id"].as_str().unwrap();

        // Caller-side read via the audit log (GET /v1/invocations/{id}).
        let get_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/invocations/{invocation_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_res.status(), StatusCode::OK);
        let get_body: Value =
            serde_json::from_slice(&get_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            get_body["tier"], "friend",
            "grant-path invocation must stamp tier='friend' on get()"
        );
        assert!(
            !get_body["grant_id"].is_null(),
            "grant-path invocation keeps grant_id non-null"
        );

        // Audit-log list view (GET /v1/invocations).
        let list_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/invocations?direction=outbound")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_body: Value =
            serde_json::from_slice(&list_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let arr = list_body.as_array().expect("array");
        let row = arr
            .iter()
            .find(|r| r["id"] == invocation_id)
            .expect("row in list");
        assert_eq!(
            row["tier"], "friend",
            "grant-path invocation must stamp tier='friend' on list()"
        );

        // Granter-side read via inbox.pull (where contexts are populated).
        let alice_user_id: uuid::Uuid = sqlx::query_scalar!(
            "SELECT owner_user_id FROM accounts a JOIN agents g ON g.account_id=a.id WHERE g.id=$1",
            f.granter_agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
        let alice_token = encode_jwt(
            &UserClaims::new(alice_user_id, format!("{alice_user_id}@t.local"), false, 1),
            "test-secret",
        )
        .unwrap();
        let inbox_res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/inbox?agent_id={}", f.granter_agent_id))
                    .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let inbox_body: Value =
            serde_json::from_slice(&inbox_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let inbox_arr = inbox_body.as_array().expect("array");
        assert_eq!(inbox_arr.len(), 1);
        assert_eq!(
            inbox_arr[0]["tier"], "friend",
            "grant-path invocation must stamp tier='friend' on inbox.pull"
        );
        // Sanity: contexts are present on the inbox response.
        assert!(!inbox_arr[0]["grant_context"].is_null());
        assert!(!inbox_arr[0]["friendship_context"].is_null());
    }
}

#[cfg(test)]
mod trust_snapshot_tests {
    //! Edge 2 from the trust-context audit (migration 0024). Audit-log
    //! endpoints must return the friendship + grant *as they were at
    //! queue time*, not as they are now. The relay snapshots both into
    //! a JSONB column on the row's INSERT, and the read path
    //! deserializes the snapshot back into the same typed contexts the
    //! inbox endpoint returns — so consumers see one shape regardless
    //! of which read path delivered it.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::legacy_v01_contract_tests::{config, seed_demo};

    /// The grant-path invoke must stamp `trust_snapshot` with both
    /// friendship + grant rows, and the audit-log endpoints must
    /// surface them as typed contexts — even after the underlying
    /// friendship / grant changes.
    #[sqlx::test(migrations = "../migrations")]
    async fn audit_log_returns_frozen_contexts_even_after_state_drift(pool: PgPool) {
        let f = seed_demo(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        // Bob (caller) → POST /v1/invoke.
        let invoke_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/invoke")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "grant_id": f.grant_id,
                            "grantee_agent_id": f.grantee_agent_id,
                            "input": {"duration_min": 30}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invoke_res.status(), StatusCode::ACCEPTED);
        let invoke_body: Value =
            serde_json::from_slice(&invoke_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let invocation_id = invoke_body["invocation_id"].as_str().unwrap();

        // Snapshot lives on the row from the moment it's queued.
        let snapshot: Option<Value> = sqlx::query_scalar!(
            "SELECT trust_snapshot FROM relay_invocations WHERE id = $1",
            invocation_id.parse::<Uuid>().unwrap(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let snapshot = snapshot.expect("grant-path invoke must populate trust_snapshot");
        assert_eq!(
            snapshot["friendship"]["status"], "accepted",
            "friendship snapshot should reflect the accepted state at queue time"
        );
        assert_eq!(
            snapshot["grant"]["status"], "active",
            "grant snapshot should reflect the active state at queue time"
        );
        assert_eq!(snapshot["grant"]["id"], f.grant_id.to_string());

        // Now drift the state: revoke the grant. The audit log must still
        // return the contexts AS THEY WERE WHEN QUEUED.
        sqlx::query!(
            "UPDATE grants SET status = 'revoked', revoked_at = now() WHERE id = $1",
            f.grant_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Audit-log get → contexts populated from the frozen snapshot.
        let get_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/invocations/{invocation_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let get_body: Value =
            serde_json::from_slice(&get_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(get_body["tier"], "friend");
        assert_eq!(
            get_body["grant_context"]["status"], "active",
            "audit log must show the grant state AT QUEUE TIME (active), \
             not the now-revoked state"
        );
        assert_eq!(get_body["grant_context"]["id"], f.grant_id.to_string());
        assert_eq!(get_body["friendship_context"]["status"], "accepted");

        // Audit-log list → same.
        let list_res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/invocations?direction=outbound")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_body: Value =
            serde_json::from_slice(&list_res.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let row = list_body
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == invocation_id)
            .expect("row present");
        assert_eq!(row["grant_context"]["status"], "active");
        assert_eq!(row["friendship_context"]["status"], "accepted");
    }
}
