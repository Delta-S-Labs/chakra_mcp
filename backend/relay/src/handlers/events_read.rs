//! Read side for the audit + usage event tables (migration 0025).
//!
//!   GET /v1/audit         — the write/audit trail for the caller's
//!                            accounts (+ their own actions).
//!   GET /v1/usage/events  — metered requests for the caller, with a
//!                            lightweight by-action total for billing.
//!
//! Both are account-scoped: a caller sees events for any account they're
//! a member of, plus anything they personally did. Cursor pagination is
//! keyed on (created_at, id) descending — newest first.

use axum::extract::{Query, State};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::state::RelayState;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Serialize, Deserialize)]
struct Cursor {
    created_at: DateTime<Utc>,
    id: Uuid,
}

fn encode_cursor(c: &Cursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(c).expect("cursor serialize"))
}

fn decode_cursor(s: &str) -> Result<Cursor, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ApiError::InvalidRequest("malformed cursor".into()))?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::InvalidRequest("malformed cursor".into()))
}

fn clamp_limit(l: Option<i64>) -> i64 {
    l.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

// ─── GET /v1/audit ───────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct AuditQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// Filter to a single resource type ('agent','friendship',…).
    pub resource_type: Option<String>,
    /// Filter to a single action ('grant.revoke', …).
    pub action: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditEventDto {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub actor_user_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<Uuid>,
    pub target_id: Option<Uuid>,
    pub summary: String,
    pub metadata: Value,
}

#[derive(Debug, Serialize)]
pub struct AuditListResponse {
    pub events: Vec<AuditEventDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub async fn audit_list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<AuditListResponse>> {
    let limit = clamp_limit(q.limit);
    let cur = q.cursor.as_deref().map(decode_cursor).transpose()?;
    let (cur_ts, cur_id) = match cur {
        Some(c) => (Some(c.created_at), Some(c.id)),
        None => (None, None),
    };

    let rows = sqlx::query!(
        r#"
        SELECT id, created_at, actor_user_id, account_id, action,
               resource_type, resource_id, target_id, summary, metadata
        FROM audit_events
        WHERE (
                account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                )
                OR actor_user_id = $1
              )
          AND ($2::text IS NULL OR resource_type = $2)
          AND ($3::text IS NULL OR action = $3)
          AND (
                $4::timestamptz IS NULL
                OR (created_at, id) < ($4, $5)
              )
        ORDER BY created_at DESC, id DESC
        LIMIT $6
        "#,
        user.user_id,
        q.resource_type,
        q.action,
        cur_ts,
        cur_id,
        limit + 1,
    )
    .fetch_all(&state.db)
    .await?;

    let has_more = rows.len() as i64 > limit;
    let page = &rows[..rows.len().min(limit as usize)];
    let next_cursor = if has_more {
        page.last().map(|r| {
            encode_cursor(&Cursor {
                created_at: r.created_at,
                id: r.id,
            })
        })
    } else {
        None
    };

    Ok(Json(AuditListResponse {
        events: page
            .iter()
            .map(|r| AuditEventDto {
                id: r.id,
                created_at: r.created_at,
                actor_user_id: r.actor_user_id,
                account_id: r.account_id,
                action: r.action.clone(),
                resource_type: r.resource_type.clone(),
                resource_id: r.resource_id,
                target_id: r.target_id,
                summary: r.summary.clone(),
                metadata: r.metadata.clone(),
            })
            .collect(),
        next_cursor,
    }))
}

// ─── GET /v1/usage/events ────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct UsageQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// 'rest' | 'mcp'
    pub surface: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UsageEventDto {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub actor_user_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub surface: String,
    pub action: String,
    pub method: String,
    pub route: String,
    pub status_code: i32,
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct UsageActionCount {
    pub action: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct UsageListResponse {
    pub events: Vec<UsageEventDto>,
    /// Totals by action over the caller's whole history — a billing
    /// preview, independent of the current page.
    pub totals_by_action: Vec<UsageActionCount>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub async fn usage_list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<UsageQuery>,
) -> ApiResult<Json<UsageListResponse>> {
    let limit = clamp_limit(q.limit);
    let cur = q.cursor.as_deref().map(decode_cursor).transpose()?;
    let (cur_ts, cur_id) = match cur {
        Some(c) => (Some(c.created_at), Some(c.id)),
        None => (None, None),
    };

    let rows = sqlx::query!(
        r#"
        SELECT id, created_at, actor_user_id, account_id, surface, action,
               method, route, status_code, ok
        FROM usage_events
        WHERE (
                account_id IN (
                    SELECT account_id FROM account_memberships WHERE user_id = $1
                )
                OR actor_user_id = $1
              )
          AND ($2::text IS NULL OR surface = $2)
          AND (
                $3::timestamptz IS NULL
                OR (created_at, id) < ($3, $4)
              )
        ORDER BY created_at DESC, id DESC
        LIMIT $5
        "#,
        user.user_id,
        q.surface,
        cur_ts,
        cur_id,
        limit + 1,
    )
    .fetch_all(&state.db)
    .await?;

    let has_more = rows.len() as i64 > limit;
    let page = &rows[..rows.len().min(limit as usize)];
    let next_cursor = if has_more {
        page.last().map(|r| {
            encode_cursor(&Cursor {
                created_at: r.created_at,
                id: r.id,
            })
        })
    } else {
        None
    };

    // Billing preview: counts by action across the caller's full history.
    let totals = sqlx::query!(
        r#"
        SELECT action, COUNT(*)::bigint AS "count!"
        FROM usage_events
        WHERE account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR actor_user_id = $1
        GROUP BY action
        ORDER BY "count!" DESC
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    let total: i64 = totals.iter().map(|t| t.count).sum();

    Ok(Json(UsageListResponse {
        events: page
            .iter()
            .map(|r| UsageEventDto {
                id: r.id,
                created_at: r.created_at,
                actor_user_id: r.actor_user_id,
                account_id: r.account_id,
                surface: r.surface.clone(),
                action: r.action.clone(),
                method: r.method.clone(),
                route: r.route.clone(),
                status_code: r.status_code,
                ok: r.ok,
            })
            .collect(),
        totals_by_action: totals
            .into_iter()
            .map(|t| UsageActionCount {
                action: t.action,
                count: t.count,
            })
            .collect(),
        total,
        next_cursor,
    }))
}
