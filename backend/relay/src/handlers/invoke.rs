//! Sync invoke + audit log.
//!
//! POST /v1/invoke
//!   The grantee's owner asks the relay to deliver `input` to the
//!   granter's webhook for `capability_name`. The relay:
//!     1. validates the grant is active and matches the (granter,
//!        grantee, capability) triple,
//!     2. checks the granter exposes an endpoint URL,
//!     3. HMAC-signs the payload with WEBHOOK_SIGNING_SECRET,
//!     4. POSTs to that URL with a 30s deadline,
//!     5. records the attempt — success, failure, timeout, or
//!        pre-flight rejection — in `relay_invocations`.
//!   The granter's response body (if 2xx + JSON) is returned to the
//!   caller as the invoke response.
//!
//! GET /v1/invocations
//!   Audit log. Members of either side (granter or grantee account)
//!   can read their own rows. Filterable by direction and agent_id.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

const INVOKE_TIMEOUT_SECS: u64 = 30;
const PREVIEW_BYTE_LIMIT: usize = 16 * 1024;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InvokeRequest {
    pub grant_id: Uuid,
    /// The agent the caller is invoking AS — must be a member of its
    /// account, must match the grant's grantee_agent_id.
    pub grantee_agent_id: Uuid,
    pub input: Value,
}

#[derive(Debug, Serialize)]
pub struct InvokeResponse {
    pub invocation_id: Uuid,
    pub status: String,
    pub http_status: Option<i32>,
    pub elapsed_ms: i32,
    pub output: Option<Value>,
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
    pub status: String,
    pub http_status: Option<i32>,
    pub elapsed_ms: i32,
    pub error_message: Option<String>,
    pub input_preview: Option<Value>,
    pub output_preview: Option<Value>,
    pub created_at: DateTime<Utc>,
    /// True when the requesting user is on the granter side.
    pub i_served: bool,
    /// True when the requesting user is on the grantee side.
    pub i_invoked: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// "outbound" (I served) | "inbound" (I invoked) | omitted for both.
    pub direction: Option<String>,
    pub agent_id: Option<Uuid>,
    pub status: Option<String>,
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

fn sign_payload(secret: &str, body: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    let sig = mac.finalize().into_bytes();
    format!("sha256={}", hex::encode(sig))
}

#[allow(clippy::too_many_arguments)]
async fn record(
    db: &PgPool,
    grant_id: Option<Uuid>,
    granter_agent_id: Option<Uuid>,
    grantee_agent_id: Option<Uuid>,
    capability_id: Option<Uuid>,
    capability_name: &str,
    invoked_by_user_id: Uuid,
    status: &str,
    http_status: Option<i32>,
    elapsed_ms: i32,
    error_message: Option<&str>,
    input_preview: Option<&Value>,
    output_preview: Option<&Value>,
) -> Result<Uuid, ApiError> {
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, http_status,
             elapsed_ms, error_message, input_preview, output_preview)
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
        http_status,
        elapsed_ms,
        error_message,
        input_preview.cloned().unwrap_or(Value::Null),
        output_preview.cloned().unwrap_or(Value::Null),
    )
    .execute(db)
    .await?;
    Ok(id)
}

// ─── POST /v1/invoke ─────────────────────────────────────
pub async fn invoke(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<InvokeRequest>,
) -> Result<(StatusCode, Json<InvokeResponse>), ApiError> {
    let started = std::time::Instant::now();
    let input_preview = truncate_for_audit(&req.input);

    // 1. Resolve the grant + agents + capability + endpoint in one shot.
    let row = sqlx::query!(
        r#"
        SELECT
            g.id as grant_id, g.status as grant_status,
            g.granter_agent_id, g.grantee_agent_id, g.capability_id,
            g.expires_at,
            ga.endpoint_url as granter_endpoint, ga.account_id as granter_account_id,
            ea.account_id as grantee_account_id,
            cap.name as capability_name
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        JOIN agent_capabilities cap ON cap.id = g.capability_id
        WHERE g.id = $1
        "#,
        req.grant_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.grantee_agent_id != req.grantee_agent_id {
        // Pre-flight audit row, then 400.
        let elapsed = started.elapsed().as_millis() as i32;
        let id = record(
            &state.db, Some(row.grant_id), Some(row.granter_agent_id),
            Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
            user.user_id, "rejected", None, elapsed,
            Some("grantee_agent_id does not match the grant"),
            Some(&input_preview), None,
        ).await?;
        return Ok((StatusCode::BAD_REQUEST, Json(InvokeResponse {
            invocation_id: id, status: "rejected".into(), http_status: None,
            elapsed_ms: elapsed, output: None,
            error: Some("grantee_agent_id does not match the grant".into()),
        })));
    }

    // Caller must be a member of the grantee's account.
    if !user_is_member(&state.db, user.user_id, row.grantee_account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Grant must be active (and not expired).
    if row.grant_status != "active" {
        let elapsed = started.elapsed().as_millis() as i32;
        let msg = format!("grant is {}; only active grants can be invoked", row.grant_status);
        let id = record(
            &state.db, Some(row.grant_id), Some(row.granter_agent_id),
            Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
            user.user_id, "rejected", None, elapsed,
            Some(&msg), Some(&input_preview), None,
        ).await?;
        return Ok((StatusCode::CONFLICT, Json(InvokeResponse {
            invocation_id: id, status: "rejected".into(), http_status: None,
            elapsed_ms: elapsed, output: None, error: Some(msg),
        })));
    }
    if let Some(exp) = row.expires_at {
        if exp <= Utc::now() {
            let elapsed = started.elapsed().as_millis() as i32;
            let msg = "grant has expired".to_string();
            let id = record(
                &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                user.user_id, "rejected", None, elapsed,
                Some(&msg), Some(&input_preview), None,
            ).await?;
            return Ok((StatusCode::CONFLICT, Json(InvokeResponse {
                invocation_id: id, status: "rejected".into(), http_status: None,
                elapsed_ms: elapsed, output: None, error: Some(msg),
            })));
        }
    }

    // Granter must expose an endpoint.
    let endpoint = match row.granter_endpoint.as_deref() {
        Some(e) if !e.is_empty() => e.to_string(),
        _ => {
            let elapsed = started.elapsed().as_millis() as i32;
            let msg = "granter agent has no endpoint_url configured".to_string();
            let id = record(
                &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                user.user_id, "rejected", None, elapsed,
                Some(&msg), Some(&input_preview), None,
            ).await?;
            return Ok((StatusCode::CONFLICT, Json(InvokeResponse {
                invocation_id: id, status: "rejected".into(), http_status: None,
                elapsed_ms: elapsed, output: None, error: Some(msg),
            })));
        }
    };

    let secret = state.config.webhook_signing_secret.as_deref().ok_or_else(|| {
        ApiError::Internal(anyhow::anyhow!(
            "WEBHOOK_SIGNING_SECRET is not configured on this relay"
        ))
    })?;

    // 2. Build + sign the webhook body.
    let webhook_body = serde_json::json!({
        "invocation": {
            "grant_id": row.grant_id,
            "granter_agent_id": row.granter_agent_id,
            "grantee_agent_id": row.grantee_agent_id,
            "capability": row.capability_name,
        },
        "input": req.input,
    });
    let body_bytes = serde_json::to_vec(&webhook_body)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("json encode failed: {e}")))?;
    let signature = sign_payload(secret, &body_bytes);

    // 3. Deliver.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(INVOKE_TIMEOUT_SECS))
        .build()
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("reqwest build failed: {e}")))?;

    let resp = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("x-chakramcp-signature", &signature)
        .header("x-chakramcp-grant", row.grant_id.to_string())
        .body(body_bytes)
        .send()
        .await;

    let elapsed_ms = started.elapsed().as_millis() as i32;

    match resp {
        Err(err) if err.is_timeout() => {
            let msg = format!("webhook timed out after {INVOKE_TIMEOUT_SECS}s");
            let id = record(
                &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                user.user_id, "timeout", None, elapsed_ms,
                Some(&msg), Some(&input_preview), None,
            ).await?;
            Ok((StatusCode::GATEWAY_TIMEOUT, Json(InvokeResponse {
                invocation_id: id, status: "timeout".into(), http_status: None,
                elapsed_ms, output: None, error: Some(msg),
            })))
        }
        Err(err) => {
            let msg = format!("webhook request failed: {err}");
            let id = record(
                &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                user.user_id, "failed", None, elapsed_ms,
                Some(&msg), Some(&input_preview), None,
            ).await?;
            Ok((StatusCode::BAD_GATEWAY, Json(InvokeResponse {
                invocation_id: id, status: "failed".into(), http_status: None,
                elapsed_ms, output: None, error: Some(msg),
            })))
        }
        Ok(r) => {
            let http_status = r.status().as_u16() as i32;
            let body = r.text().await.unwrap_or_default();
            let parsed: Option<Value> = if body.is_empty() {
                None
            } else {
                serde_json::from_str(&body).ok()
            };

            if (200..300).contains(&http_status) {
                let preview = parsed.as_ref().map(truncate_for_audit);
                let id = record(
                    &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                    Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                    user.user_id, "succeeded", Some(http_status), elapsed_ms,
                    None, Some(&input_preview), preview.as_ref(),
                ).await?;
                Ok((StatusCode::OK, Json(InvokeResponse {
                    invocation_id: id, status: "succeeded".into(),
                    http_status: Some(http_status), elapsed_ms, output: parsed, error: None,
                })))
            } else {
                let msg = format!("webhook returned HTTP {http_status}");
                let body_value = parsed
                    .clone()
                    .unwrap_or_else(|| Value::String(body.chars().take(2048).collect()));
                let preview = truncate_for_audit(&body_value);
                let id = record(
                    &state.db, Some(row.grant_id), Some(row.granter_agent_id),
                    Some(row.grantee_agent_id), Some(row.capability_id), &row.capability_name,
                    user.user_id, "failed", Some(http_status), elapsed_ms,
                    Some(&msg), Some(&input_preview), Some(&preview),
                ).await?;
                Ok((StatusCode::BAD_GATEWAY, Json(InvokeResponse {
                    invocation_id: id, status: "failed".into(),
                    http_status: Some(http_status), elapsed_ms, output: parsed, error: Some(msg),
                })))
            }
        }
    }
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
        if !matches!(s, "rejected" | "succeeded" | "failed" | "timeout") {
            return Err(ApiError::InvalidRequest("invalid status".into()));
        }
    }

    let want_outbound = matches!(direction, "all" | "outbound");
    let want_inbound = matches!(direction, "all" | "inbound");

    let rows = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status, i.http_status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",
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
        WHERE
            (
                ($2::boolean AND ga.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                ))
             OR ($3::boolean AND ea.account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                ))
            )
            AND ($4::uuid IS NULL OR i.granter_agent_id = $4 OR i.grantee_agent_id = $4)
            AND ($5::text IS NULL OR i.status = $5)
        ORDER BY i.created_at DESC
        LIMIT 200
        "#,
        user.user_id,
        want_outbound,
        want_inbound,
        q.agent_id,
        q.status,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| InvocationDto {
                id: r.id,
                grant_id: r.grant_id,
                granter_agent_id: r.granter_agent_id,
                granter_display_name: r.granter_display_name,
                grantee_agent_id: r.grantee_agent_id,
                grantee_display_name: r.grantee_display_name,
                capability_id: r.capability_id,
                capability_name: r.capability_name,
                status: r.status,
                http_status: r.http_status,
                elapsed_ms: r.elapsed_ms,
                error_message: r.error_message,
                input_preview: r.input_preview,
                output_preview: r.output_preview,
                created_at: r.created_at,
                i_served: r.i_served.unwrap_or(false),
                i_invoked: r.i_invoked.unwrap_or(false),
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
    let r = sqlx::query!(
        r#"
        SELECT
            i.id, i.grant_id, i.granter_agent_id, i.grantee_agent_id,
            i.capability_id, i.capability_name, i.status, i.http_status,
            i.elapsed_ms, i.error_message, i.input_preview, i.output_preview,
            i.created_at,
            ga.display_name as "granter_display_name?",
            ea.display_name as "grantee_display_name?",
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
        WHERE i.id = $2
        "#,
        user.user_id,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let i_served = r.i_served.unwrap_or(false);
    let i_invoked = r.i_invoked.unwrap_or(false);
    if !i_served && !i_invoked {
        return Err(ApiError::NotFound);
    }

    Ok(Json(InvocationDto {
        id: r.id,
        grant_id: r.grant_id,
        granter_agent_id: r.granter_agent_id,
        granter_display_name: r.granter_display_name,
        grantee_agent_id: r.grantee_agent_id,
        grantee_display_name: r.grantee_display_name,
        capability_id: r.capability_id,
        capability_name: r.capability_name,
        status: r.status,
        http_status: r.http_status,
        elapsed_ms: r.elapsed_ms,
        error_message: r.error_message,
        input_preview: r.input_preview,
        output_preview: r.output_preview,
        created_at: r.created_at,
        i_served,
        i_invoked,
    }))
}
