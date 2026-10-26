//! Calls: two-way sessions between two agents that carry messages and
//! capability requests (spec `docs/superpowers/specs/2026-10-25-calls-design.md`).
//!
//! A call is a row in `calls`; its transcript is `call_entries`, ordered by
//! `seq`; `call_deliveries` says who still has to pull each entry. Everything
//! is behind `CALLS_ENABLED` (`config`): off, there are no routes and no
//! workers.

pub mod access;
pub mod config;
pub mod feed;
pub mod lifecycle;
pub mod link;
pub mod mcp;
pub mod messages;
pub mod purge;
pub mod read;
pub mod workers;

#[cfg(test)]
mod tests;

use axum::http::StatusCode;
use chakramcp_shared::error::ApiError;
use chakramcp_shared::system_one::Relationship;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub use config::CallsConfig;

/// The largest message, its text and data serialized together (spec §5).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;
pub const MAX_TOPIC_CHARS: usize = 200;
pub const MAX_REASON_CHARS: usize = 500;
/// The idle timeout's range and default (spec §3.2).
pub const MIN_IDLE_SECS: i64 = 3_600;
pub const MAX_IDLE_SECS: i64 = 604_800;
pub const DEFAULT_IDLE_SECS: i64 = 86_400;
/// Messages one sender may post to one call per minute (spec §3.4).
pub const MESSAGES_PER_MINUTE: i64 = 30;

// ─── Errors (spec §12) ───────────────────────────────────

pub fn bad_terms(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::BAD_REQUEST, "chk.calls.bad_terms", message)
}

pub fn not_active(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::CONFLICT, "chk.calls.not_active", message)
}

pub fn busy(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::CONFLICT, "chk.calls.busy", message)
}

pub fn push_unsupported() -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        "chk.calls.push_unsupported",
        "push-mode agents can't make or take calls yet",
    )
}

// ─── A call and its parties ──────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Caller,
    Callee,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Caller => "caller",
            Role::Callee => "callee",
        }
    }

    pub fn other(self) -> Role {
        match self {
            Role::Caller => Role::Callee,
            Role::Callee => Role::Caller,
        }
    }
}

/// One side of a call, as it stands now.
#[derive(Debug, Clone, Serialize)]
pub struct Party {
    /// None once the agent is deleted.
    pub agent_id: Option<Uuid>,
    /// The agent's current account; the account at invite once it's gone.
    pub account_id: Uuid,
    pub display_name: Option<String>,
    pub account_slug: Option<String>,
    #[serde(skip)]
    pub gone: bool,
}

/// A call, read with its parties' current accounts (spec §13: accounts
/// follow the agents).
#[derive(Debug, Clone)]
pub struct Call {
    pub id: Uuid,
    pub state: String,
    pub memory: String,
    pub memory_features: Value,
    pub ttl_secs: i32,
    pub idle_secs: i32,
    pub topic: Option<String>,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub ended_by: Option<String>,
    pub purge_at: Option<DateTime<Utc>>,
    pub purged_at: Option<DateTime<Utc>>,
    pub message_count: i32,
    pub caller: Party,
    pub callee: Party,
}

impl Call {
    /// Which side `agent_id` is on, if either.
    pub fn role_of(&self, agent_id: Uuid) -> Option<Role> {
        if self.caller.agent_id == Some(agent_id) {
            Some(Role::Caller)
        } else if self.callee.agent_id == Some(agent_id) {
            Some(Role::Callee)
        } else {
            None
        }
    }

    pub fn party(&self, role: Role) -> &Party {
        match role {
            Role::Caller => &self.caller,
            Role::Callee => &self.callee,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state.as_str(), "ringing" | "active")
    }

    /// The parties still there to deliver to.
    pub fn live_agents(&self) -> Vec<Uuid> {
        [&self.caller, &self.callee]
            .into_iter()
            .filter(|p| !p.gone)
            .filter_map(|p| p.agent_id)
            .collect()
    }

    /// The call object of spec §11.
    pub fn dto(&self) -> Value {
        json!({
            "id": self.id,
            "state": self.state,
            "caller": self.caller,
            "callee": self.callee,
            "memory": self.memory,
            "memory_features": self.memory_features,
            "ttl_secs": self.ttl_secs,
            "idle_secs": self.idle_secs,
            "topic": self.topic,
            "created_at": self.created_at,
            "answered_at": self.answered_at,
            "ended_at": self.ended_at,
            "ended_by": self.ended_by,
            "purge_at": self.purge_at,
            "purged_at": self.purged_at,
            "message_count": self.message_count,
        })
    }
}

/// A call row joined with both agents and their accounts.
pub(crate) struct CallRow {
    pub id: Uuid,
    pub state: String,
    pub memory: String,
    pub memory_features: Value,
    pub ttl_secs: i32,
    pub idle_secs: i32,
    pub topic: Option<String>,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub ended_by: Option<String>,
    pub purge_at: Option<DateTime<Utc>>,
    pub purged_at: Option<DateTime<Utc>>,
    pub message_count: i32,
    pub caller_agent_id: Option<Uuid>,
    pub callee_agent_id: Option<Uuid>,
    pub caller_account_id: Uuid,
    pub callee_account_id: Uuid,
    pub caller_current_account: Option<Uuid>,
    pub caller_name: Option<String>,
    pub caller_slug: Option<String>,
    pub caller_gone: Option<bool>,
    pub callee_current_account: Option<Uuid>,
    pub callee_name: Option<String>,
    pub callee_slug: Option<String>,
    pub callee_gone: Option<bool>,
}

impl From<CallRow> for Call {
    fn from(r: CallRow) -> Self {
        let party = |agent_id: Option<Uuid>,
                     current: Option<Uuid>,
                     at_invite: Uuid,
                     name: Option<String>,
                     slug: Option<String>,
                     gone: Option<bool>| Party {
            agent_id,
            account_id: current.unwrap_or(at_invite),
            display_name: name,
            account_slug: slug,
            gone: agent_id.is_none() || gone.unwrap_or(true),
        };
        Call {
            caller: party(
                r.caller_agent_id,
                r.caller_current_account,
                r.caller_account_id,
                r.caller_name,
                r.caller_slug,
                r.caller_gone,
            ),
            callee: party(
                r.callee_agent_id,
                r.callee_current_account,
                r.callee_account_id,
                r.callee_name,
                r.callee_slug,
                r.callee_gone,
            ),
            id: r.id,
            state: r.state,
            memory: r.memory,
            memory_features: r.memory_features,
            ttl_secs: r.ttl_secs,
            idle_secs: r.idle_secs,
            topic: r.topic,
            created_at: r.created_at,
            answered_at: r.answered_at,
            ended_at: r.ended_at,
            ended_by: r.ended_by,
            purge_at: r.purge_at,
            purged_at: r.purged_at,
            message_count: r.message_count,
        }
    }
}

/// A call by id, or `None`.
pub async fn load_call(db: &PgPool, id: Uuid) -> Result<Option<Call>, ApiError> {
    let row = sqlx::query_as!(
        CallRow,
        r#"
        SELECT c.id, c.state, c.memory, c.memory_features, c.ttl_secs, c.idle_secs,
               c.topic, c.created_at, c.answered_at, c.ended_at, c.ended_by,
               c.purge_at, c.purged_at, c.message_count,
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
         WHERE c.id = $1
        "#,
        id,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(Call::from))
}

/// The two agents' accepted friendship, if any: what `off_relationship`
/// judges their messages against (spec §5.1).
pub struct Friendship {
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
}

impl Friendship {
    pub fn relationship(&self) -> Relationship<'_> {
        Relationship {
            proposer_message: self.proposer_message.as_deref(),
            response_message: self.response_message.as_deref(),
        }
    }
}

pub async fn friendship(db: &PgPool, a: Uuid, b: Uuid) -> Result<Option<Friendship>, ApiError> {
    Ok(sqlx::query_as!(
        Friendship,
        r#"
        SELECT proposer_message, response_message
          FROM friendships
         WHERE status = 'accepted'
           AND ((proposer_agent_id = $1 AND target_agent_id = $2)
             OR (proposer_agent_id = $2 AND target_agent_id = $1))
         ORDER BY updated_at DESC
         LIMIT 1
        "#,
        a,
        b,
    )
    .fetch_optional(db)
    .await?)
}

// ─── The transcript ──────────────────────────────────────

/// A transcript entry as stored.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: Uuid,
    pub call_id: Uuid,
    pub seq: i32,
    pub kind: String,
    pub from_agent_id: Option<Uuid>,
    pub text: Option<String>,
    pub data: Option<Value>,
    pub blocked: bool,
    pub safety: Option<Value>,
    pub created_at: DateTime<Utc>,
    /// For `invocation` entries: the invocation, and its capability and
    /// status as they are now (read by join).
    pub invocation_id: Option<Uuid>,
    pub invocation_capability: Option<String>,
    pub invocation_status: Option<String>,
}

impl Entry {
    /// The entry object of spec §11; `safety` omitted when empty.
    pub fn dto(&self) -> Value {
        let mut v = json!({
            "seq": self.seq,
            "kind": self.kind,
            "from_agent_id": self.from_agent_id,
            "text": self.text,
            "data": self.data,
            "blocked": self.blocked,
            "created_at": self.created_at,
        });
        if let Some(s) = self
            .safety
            .as_ref()
            .filter(|s| s.as_array().is_some_and(|a| !a.is_empty()))
        {
            v["safety"] = s.clone();
        }
        if let Some(id) = self.invocation_id {
            v["invocation"] = json!({
                "id": id,
                "capability": self.invocation_capability,
                "status": self.invocation_status,
            });
        }
        v
    }
}

/// An entry to append.
pub struct NewEntry<'a> {
    pub kind: &'a str,
    pub from: Option<Uuid>,
    pub text: Option<&'a str>,
    pub data: Option<&'a Value>,
    pub blocked: bool,
    pub size_bytes: i32,
    pub safety: Option<Value>,
    pub invocation_id: Option<Uuid>,
}

impl<'a> NewEntry<'a> {
    /// A system event (`accepted`, `ended`, …) with no content.
    pub fn event(kind: &'a str, from: Option<Uuid>) -> Self {
        Self {
            kind,
            from,
            text: None,
            data: None,
            blocked: false,
            size_bytes: 0,
            safety: None,
            invocation_id: None,
        }
    }
}

/// Append an entry with the call's next `seq`, to be delivered to `to`.
/// Run it in the transaction that changes the call: the `seq` update takes
/// the call's row lock, which orders entries per call.
pub async fn append(
    conn: &mut PgConnection,
    call_id: Uuid,
    entry: NewEntry<'_>,
    to: &[Uuid],
) -> Result<Entry, ApiError> {
    let seq = sqlx::query_scalar!(
        "UPDATE calls SET last_seq = last_seq + 1 WHERE id = $1 RETURNING last_seq",
        call_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    let id = Uuid::now_v7();
    let created_at = sqlx::query_scalar!(
        r#"
        INSERT INTO call_entries
            (id, call_id, seq, kind, from_agent_id, text, data, blocked, size_bytes, safety,
             invocation_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING created_at
        "#,
        id,
        call_id,
        seq,
        entry.kind,
        entry.from,
        entry.text,
        entry.data,
        entry.blocked,
        entry.size_bytes,
        entry.safety,
        entry.invocation_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    if !to.is_empty() {
        sqlx::query!(
            r#"
            INSERT INTO call_deliveries (entry_id, agent_id)
            SELECT $1, unnest($2::uuid[])
            "#,
            id,
            to,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(Entry {
        id,
        call_id,
        seq,
        kind: entry.kind.to_owned(),
        from_agent_id: entry.from,
        text: entry.text.map(str::to_owned),
        data: entry.data.cloned(),
        blocked: entry.blocked,
        safety: entry.safety,
        created_at,
        invocation_id: entry.invocation_id,
        invocation_capability: None,
        invocation_status: None,
    })
}

/// Queue a message's charge, paid by its sender's account (spec §9).
pub async fn queue_message_charge(
    conn: &mut PgConnection,
    entry_id: Uuid,
    account_id: Uuid,
    call_id: Uuid,
    cost_mc: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        r#"
        INSERT INTO call_charge_queue (id, account_id, call_id, kind, cost_mc)
        VALUES ($1, $2, $3, 'message', $4)
        "#,
        entry_id,
        account_id,
        call_id,
        cost_mc,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
