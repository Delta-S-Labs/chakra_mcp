//! The calls feed (spec §6.1): `GET /v1/inbox/calls` claims an agent's
//! undelivered entries, oldest first, each delivered once.

use std::collections::HashMap;

use chakramcp_shared::error::ApiError;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{access, load_call, Entry};
use crate::auth::AuthUser;
use crate::state::RelayState;

/// The most items one pull returns.
pub const MAX_ITEMS: i64 = 25;

#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    pub agent_id: Uuid,
    pub limit: Option<i64>,
}

pub async fn claim(
    state: &RelayState,
    user: &AuthUser,
    q: FeedQuery,
) -> Result<Vec<Value>, ApiError> {
    let acting = access::act_as(state, user, q.agent_id).await?;
    // Pulling calls counts as being online, as pulling the inbox does.
    state.heartbeats.record(acting.agent_id);
    let limit = q.limit.unwrap_or(MAX_ITEMS).clamp(1, MAX_ITEMS);

    let mut tx = state.db.begin().await?;
    let claimed: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        WITH picked AS (
            SELECT d.entry_id
              FROM call_deliveries d
              JOIN call_entries e ON e.id = d.entry_id
             WHERE d.agent_id = $1 AND d.delivered_at IS NULL
             ORDER BY d.created_at, e.seq
             LIMIT $2
               FOR UPDATE OF d SKIP LOCKED
        )
        UPDATE call_deliveries d SET delivered_at = now()
          FROM picked p
         WHERE d.entry_id = p.entry_id AND d.agent_id = $1
        RETURNING d.entry_id
        "#,
        acting.agent_id,
        limit,
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    if claimed.is_empty() {
        return Ok(Vec::new());
    }

    let entries = sqlx::query_as!(
        Entry,
        r#"
        SELECT id, call_id, seq, kind, from_agent_id, text, data, blocked, safety, created_at
          FROM call_entries
         WHERE id = ANY($1)
         ORDER BY created_at, call_id, seq
        "#,
        &claimed,
    )
    .fetch_all(&state.db)
    .await?;
    let mut calls = HashMap::new();
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        if let std::collections::hash_map::Entry::Vacant(slot) = calls.entry(entry.call_id) {
            if let Some(call) = load_call(&state.db, entry.call_id).await? {
                slot.insert(call.dto());
            }
        }
        let Some(call) = calls.get(&entry.call_id) else {
            continue;
        };
        items.push(json!({ "call": call, "entry": entry.dto() }));
    }
    Ok(items)
}
