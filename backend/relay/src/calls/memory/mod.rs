//! Call memory (spec §8): Jev tags the call's messages in the background
//! (`tagger`), code composes a bundle from them (here), and agents can ask
//! the call's memory a question (`recall`). The Jev requests and the bundle's
//! rules are in `chakramcp_shared::calls_memory`.

pub mod recall;
pub mod tagger;

use axum::http::StatusCode;
use chakramcp_shared::calls_memory::{self, BundleEntry, Invocation, Tags};
use chakramcp_shared::error::ApiError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::{access, load_call, Call};
use crate::auth::AuthUser;
use crate::state::RelayState;

/// Call memory's TypeSafe client (`TYPESAFE_AI_*`), when calls and memory
/// are both on. Memory doesn't need `SYSTEM_ONE_CHECKS`; it needs the key.
pub fn client(cfg: &super::CallsConfig) -> Option<chakramcp_shared::system_one::ComplianceChecker> {
    if !(cfg.enabled && cfg.memory_enabled) {
        return None;
    }
    let client = chakramcp_shared::system_one::ComplianceChecker::client_from_env();
    if client.is_none() {
        tracing::error!(
            "CALLS_MEMORY_ENABLED is on but TYPESAFE_AI_KEY is not set: call memory DISABLED"
        );
    }
    client
}

/// A call without memory, or memory switched off when it was placed.
pub fn unavailable(message: impl Into<String>) -> ApiError {
    ApiError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "chk.calls.memory_unavailable",
        message,
    )
}

/// The call's memory bundle (spec §8.3), from its stored messages and
/// invocation entries.
pub async fn load_bundle(state: &RelayState, call: &Call) -> Result<Value, ApiError> {
    let rows = sqlx::query!(
        r#"
        SELECT e.seq, e.kind, e.from_agent_id, e.text, e.tags, e.tag_state, e.created_at,
               e.invocation_id,
               i.capability_name AS "capability?", i.status AS "status?",
               i.input_preview AS "input?", i.output_preview AS "output?"
          FROM call_entries e
          LEFT JOIN relay_invocations i ON i.id = e.invocation_id
         WHERE e.call_id = $1
           AND ((e.kind = 'message' AND NOT e.blocked AND e.text IS NOT NULL)
             OR e.kind = 'invocation')
         ORDER BY e.seq
        "#,
        call.id,
    )
    .fetch_all(&state.db)
    .await?;
    let entries: Vec<BundleEntry> = rows
        .into_iter()
        .map(|r| BundleEntry {
            seq: r.seq,
            from: r.from_agent_id,
            at: r.created_at,
            text: r.text,
            tags: r
                .tags
                .filter(|_| r.tag_state.as_deref() == Some("done"))
                .and_then(|t| serde_json::from_value::<Tags>(t).ok()),
            invocation: r.invocation_id.map(|id| Invocation {
                id,
                capability: r.capability.unwrap_or_default(),
                status: r.status.unwrap_or_default(),
                input: r.input,
                output: r.output,
            }),
        })
        .collect();
    Ok(calls_memory::bundle(
        &entries,
        state.calls.tag_threshold,
        call.tags_on(),
    ))
}

/// `GET /v1/calls/{id}/memory`: for either party and owners.
pub async fn get(state: &RelayState, user: &AuthUser, call_id: Uuid) -> Result<Value, ApiError> {
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    access::can_read(state, user, &call).await?;
    if call.memory != "call" {
        return Err(unavailable("this call doesn't keep memory (memory: own)"));
    }
    load_bundle(state, &call).await
}

/// Record a memory request's cost and charge it to the caller's account
/// (spec §9): a log row, a queued charge, and the call's running total.
pub async fn meter(
    conn: &mut sqlx::PgConnection,
    state: &RelayState,
    call: &Call,
    asking_agent: Option<Uuid>,
    kind: &str,
    input_tokens: i64,
) -> Result<(), ApiError> {
    let cost = state.calls.memory_cost_mc(input_tokens);
    sqlx::query!(
        r#"
        INSERT INTO call_memory_requests (id, call_id, agent_id, kind, input_tokens, cost_mc)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
        Uuid::now_v7(),
        call.id,
        asking_agent,
        kind,
        i32::try_from(input_tokens).unwrap_or(i32::MAX),
        cost,
    )
    .execute(&mut *conn)
    .await?;
    if cost > 0 {
        sqlx::query!(
            r#"
            INSERT INTO call_charge_queue (id, account_id, call_id, kind, cost_mc)
            VALUES ($1, $2, $3, 'memory', $4)
            "#,
            Uuid::now_v7(),
            call.caller.account_id,
            call.id,
            cost,
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "UPDATE calls SET memory_cost_mc = memory_cost_mc + $2 WHERE id = $1",
            call.id,
            cost,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// A message's side, from who sent it.
pub fn side_of(call: &Call, from: Option<Uuid>) -> calls_memory::Side {
    if from.is_some() && from == call.caller.agent_id {
        calls_memory::Side::Caller
    } else {
        calls_memory::Side::Callee
    }
}

/// A stored entry's text and when it was sent, by `seq`.
pub(crate) struct Stored {
    pub seq: i32,
    pub from_agent_id: Option<Uuid>,
    pub text: Option<String>,
    pub created_at: DateTime<Utc>,
}
