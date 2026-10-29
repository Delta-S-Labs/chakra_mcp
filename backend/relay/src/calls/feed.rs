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
    /// Seconds to wait for events when there are none (waking pull agents
    /// §6). MCP's `pull_calls` names it `wait_seconds`.
    #[serde(alias = "wait_seconds")]
    pub wait: Option<u32>,
}

/// Claim the agent's undelivered entries, waiting up to `q.wait` seconds
/// when there are none. Returns them with the seconds honoured.
pub async fn claim(
    state: &RelayState,
    user: &AuthUser,
    q: FeedQuery,
) -> Result<(Vec<Value>, u32), ApiError> {
    let acting = access::act_as(state, user, q.agent_id).await?;
    // Pulling calls counts as being online, as pulling the inbox does.
    state.heartbeats.record(acting.agent_id);
    let limit = q.limit.unwrap_or(MAX_ITEMS).clamp(1, MAX_ITEMS);

    let (claimed, waited) = crate::inbox_wait::claim_waiting(
        state,
        acting.agent_id,
        chakramcp_shared::wake::Feed::Calls,
        q.wait,
        || claim_ids(state, acting.agent_id, limit),
    )
    .await?;
    if claimed.is_empty() {
        return Ok((Vec::new(), waited));
    }
    Ok((describe(state, &claimed).await?, waited))
}

/// Mark up to `limit` of `agent`'s undelivered entries delivered.
async fn claim_ids(state: &RelayState, agent: Uuid, limit: i64) -> Result<Vec<Uuid>, ApiError> {
    Ok(sqlx::query_scalar!(
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
        agent,
        limit,
    )
    .fetch_all(&state.db)
    .await?)
}

/// The claimed entries as feed items.
async fn describe(state: &RelayState, claimed: &[Uuid]) -> Result<Vec<Value>, ApiError> {
    let entries = sqlx::query_as!(
        Entry,
        r#"
        SELECT e.id, e.call_id, e.seq, e.kind, e.from_agent_id, e.text, e.data,
               e.blocked, e.safety, e.created_at, e.invocation_id,
               i.capability_name AS "invocation_capability?",
               i.status AS "invocation_status?"
          FROM call_entries e
          LEFT JOIN relay_invocations i ON i.id = e.invocation_id
         WHERE e.id = ANY($1)
         ORDER BY e.created_at, e.call_id, e.seq
        "#,
        claimed,
    )
    .fetch_all(&state.db)
    .await?;
    let mut calls = HashMap::new();
    let mut items = Vec::with_capacity(entries.len());
    // The newest message item per call, for its memory bundle.
    let mut newest: HashMap<Uuid, usize> = HashMap::new();
    for entry in entries {
        if let std::collections::hash_map::Entry::Vacant(slot) = calls.entry(entry.call_id) {
            if let Some(call) = load_call(&state.db, entry.call_id).await? {
                slot.insert(call);
            }
        }
        let Some(call) = calls.get(&entry.call_id) else {
            continue;
        };
        if entry.kind == "message" {
            newest.insert(entry.call_id, items.len());
        }
        items.push(json!({ "call": call.dto(), "entry": entry.dto() }));
    }
    // One bundle per memory call per pull, on its newest message (spec §6.1).
    for (call_id, i) in newest {
        let call = &calls[&call_id];
        if call.memory == "call" {
            items[i]["memory"] = super::memory::load_bundle(state, call).await?;
        }
    }
    Ok(items)
}
