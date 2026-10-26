//! Reading calls: one, or a list (spec §11). Parties and owners alike.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chakramcp_shared::error::ApiError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{access, load_call, Call, CallRow};
use crate::auth::AuthUser;
use crate::state::RelayState;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// `GET /v1/calls/{id}`.
pub async fn get(state: &RelayState, user: &AuthUser, id: Uuid) -> Result<Value, ApiError> {
    let call = load_call(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    access::can_read(state, user, &call).await?;
    Ok(call.dto())
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// One agent's calls; without it, the calls of every agent in the
    /// user's accounts.
    pub agent_id: Option<Uuid>,
    pub state: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    created_at: DateTime<Utc>,
    id: Uuid,
}

/// `GET /v1/calls`: newest first, `{calls, next_cursor?}`.
pub async fn list(state: &RelayState, user: &AuthUser, q: ListQuery) -> Result<Value, ApiError> {
    if let Some(s) = q.state.as_deref() {
        if !matches!(s, "ringing" | "active" | "declined" | "missed" | "ended") {
            return Err(ApiError::InvalidRequest(
                "state must be ringing|active|declined|missed|ended".into(),
            ));
        }
    }
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let cursor = match q.cursor.as_deref() {
        Some(c) => Some(decode(c)?),
        None => None,
    };
    let accounts: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT account_id FROM account_memberships WHERE user_id = $1",
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    let rows = sqlx::query_as!(
        CallRow,
        r#"
        SELECT c.id, c.state, c.memory, c.memory_features, c.ttl_secs, c.idle_secs,
               c.topic, c.created_at, c.answered_at, c.ended_at, c.ended_by,
               c.purge_at, c.purged_at, c.message_count, c.last_message_at,
               c.caller_agent_id, c.callee_agent_id,
               c.caller_account_id, c.callee_account_id,
               ca.account_id AS "caller_current_account?",
               ca.display_name AS "caller_name?",
               cacc.slug AS "caller_slug?",
               (ca.tombstoned_at IS NOT NULL) AS "caller_gone?",
               cb.account_id AS "callee_current_account?",
               cb.display_name AS "callee_name?",
               cbacc.slug AS "callee_slug?",
               (cb.tombstoned_at IS NOT NULL) AS "callee_gone?"
          FROM calls c
          LEFT JOIN agents ca ON ca.id = c.caller_agent_id
          LEFT JOIN accounts cacc ON cacc.id = ca.account_id
          LEFT JOIN agents cb ON cb.id = c.callee_agent_id
          LEFT JOIN accounts cbacc ON cbacc.id = cb.account_id
         WHERE (COALESCE(ca.account_id, c.caller_account_id) = ANY($1)
             OR COALESCE(cb.account_id, c.callee_account_id) = ANY($1))
           AND ($2::uuid IS NULL OR c.caller_agent_id = $2 OR c.callee_agent_id = $2)
           AND ($3::text IS NULL OR c.state = $3)
           AND ($4::timestamptz IS NULL OR (c.created_at, c.id) < ($4, $5))
         ORDER BY c.created_at DESC, c.id DESC
         LIMIT $6
        "#,
        &accounts,
        q.agent_id,
        q.state,
        cursor.as_ref().map(|c| c.created_at),
        cursor.as_ref().map(|c| c.id).unwrap_or_default(),
        limit + 1,
    )
    .fetch_all(&state.db)
    .await?;
    let mut calls: Vec<Call> = rows.into_iter().map(Call::from).collect();
    let next_cursor = if calls.len() as i64 > limit {
        calls.truncate(limit as usize);
        calls.last().map(|c| {
            encode(&Cursor {
                created_at: c.created_at,
                id: c.id,
            })
        })
    } else {
        None
    };
    let mut body = json!({ "calls": calls.iter().map(Call::dto).collect::<Vec<_>>() });
    if let Some(c) = next_cursor {
        body["next_cursor"] = json!(c);
    }
    Ok(body)
}

fn encode(c: &Cursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(c).expect("cursor serialize"))
}

fn decode(s: &str) -> Result<Cursor, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ApiError::InvalidRequest("malformed cursor".into()))?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::InvalidRequest("malformed cursor".into()))
}
