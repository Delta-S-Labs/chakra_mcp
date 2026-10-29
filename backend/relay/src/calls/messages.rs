//! Messages on a call (spec §5) and the transcript's history (spec §8.5).

use axum::http::StatusCode;
use chakramcp_shared::error::ApiError;
use chakramcp_shared::system_one::{Block, Relationship};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{
    access, append, friendship, load_call, not_active, queue_message_charge, Call, Entry, NewEntry,
    MAX_MESSAGE_BYTES, MESSAGES_PER_MINUTE,
};
use crate::auth::AuthUser;
use crate::state::RelayState;

/// A message after screening.
pub struct Screened {
    pub text: String,
    pub data: Option<Value>,
    /// Message safety's findings, when there are any.
    pub findings: Option<Value>,
    pub block: Option<Block>,
}

/// The serialized size of a message, checked against [`MAX_MESSAGE_BYTES`]
/// (413).
pub fn size_of(text: &str, data: Option<&Value>) -> Result<i32, ApiError> {
    let n = serde_json::to_vec(&json!({ "text": text, "data": data }))
        .map_err(|e| ApiError::Internal(e.into()))?
        .len();
    if n > MAX_MESSAGE_BYTES {
        return Err(ApiError::PayloadTooLarge {
            code: "payload_too_large",
            message: format!("a message is at most {MAX_MESSAGE_BYTES} bytes (got {n})"),
        });
    }
    Ok(n as i32)
}

/// Screen a message on the `message` channel when message safety is on
/// (spec §5.1); otherwise it passes unchanged.
pub async fn screen(
    state: &RelayState,
    text: &str,
    data: Option<&Value>,
    relationship: Option<Relationship<'_>>,
) -> Result<Screened, ApiError> {
    let unchanged = || Screened {
        text: text.to_owned(),
        data: data.cloned(),
        findings: None,
        block: None,
    };
    let Some(checker) = state.compliance.as_deref().filter(|c| c.safety_enabled()) else {
        return Ok(unchanged());
    };
    // A message is at most 16 KiB, far under screening's 1 MiB cap.
    let s = checker
        .screen_message(text, data, relationship)
        .await
        .map_err(|_| ApiError::PayloadTooLarge {
            code: "chk.safety.too_large",
            message: "this message is too large to screen".into(),
        })?;
    if !s.findings.is_empty() {
        tracing::info!(findings = ?s.findings, blocked = s.block.is_some(), "safety screening: message");
    }
    let findings = (!s.findings.is_empty()).then(|| json!(s.findings));
    if s.block.is_some() {
        return Ok(Screened {
            findings,
            block: s.block,
            ..unchanged()
        });
    }
    Ok(Screened {
        text: s.value["text"].as_str().unwrap_or(text).to_owned(),
        data: s.value.get("data").cloned(),
        findings,
        block: None,
    })
}

/// The friendship between a call's two agents, if both are still there.
pub async fn relationship_of(
    state: &RelayState,
    call: &Call,
) -> Result<Option<super::Friendship>, ApiError> {
    match (call.caller.agent_id, call.callee.agent_id) {
        (Some(a), Some(b)) => friendship(&state.db, a, b).await,
        _ => Ok(None),
    }
}

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub agent_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub data: Option<Value>,
}

/// `POST /v1/calls/{id}/messages`.
pub async fn send(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    req: SendRequest,
) -> Result<Entry, ApiError> {
    let acting = access::act_as(state, user, req.agent_id).await?;
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let role = call.role_of(acting.agent_id).ok_or(ApiError::NotFound)?;
    if call.state != "active" {
        return Err(not_active(format!("this call is {}", call.state)));
    }
    let size = size_of(&req.text, req.data.as_ref())?;

    // Two agents can reply to each other forever (spec §3.4).
    let recent = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "n!" FROM call_entries
         WHERE call_id = $1 AND from_agent_id = $2 AND kind = 'message'
           AND created_at > now() - interval '1 minute'
        "#,
        call.id,
        acting.agent_id,
    )
    .fetch_one(&state.db)
    .await?;
    if recent >= MESSAGES_PER_MINUTE {
        return Err(ApiError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            "chk.calls.slow_down",
            format!("at most {MESSAGES_PER_MINUTE} messages a minute to one call"),
        ));
    }
    if call.message_count >= state.calls.max_messages {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            "chk.calls.full",
            format!(
                "this call holds its limit of {} messages: start a new call",
                state.calls.max_messages
            ),
        ));
    }
    let limits = state.enforce_limits(acting.account_id, "calls").await;
    if let Some(outcome) = limits.refusal {
        return Err(state.refusal_error(outcome));
    }

    let friends = relationship_of(state, &call).await?;
    let screened = screen(
        state,
        &req.text,
        req.data.as_ref(),
        friends.as_ref().map(|f| f.relationship()),
    )
    .await?;

    let mut tx = state.db.begin().await?;
    // Locks the call, and checks it's still active.
    let open = sqlx::query_scalar!(
        "SELECT id FROM calls WHERE id = $1 AND state = 'active' FOR UPDATE",
        call.id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if open.is_none() {
        return Err(not_active("this call has ended"));
    }

    if let Some(block) = screened.block {
        // A stand-in, so the transcript shows something was refused; no
        // content, no delivery, no charge.
        let mut entry = NewEntry::event("message", Some(acting.agent_id));
        entry.blocked = true;
        entry.size_bytes = size;
        entry.safety = screened.findings;
        append(&mut tx, call.id, entry, &[]).await?;
        tx.commit().await?;
        return Err(ApiError::Policy {
            code: block.code,
            message: block.reason,
        });
    }

    let other = call.party(role.other());
    let to: Vec<Uuid> = other.agent_id.filter(|_| !other.gone).into_iter().collect();
    let entry = append(
        &mut tx,
        call.id,
        NewEntry {
            kind: "message",
            from: Some(acting.agent_id),
            text: Some(&screened.text),
            data: screened.data.as_ref(),
            blocked: false,
            size_bytes: size,
            safety: screened.findings,
            invocation_id: None,
            tag: call.tags_on(),
        },
        &to,
    )
    .await?;
    queue_message_charge(
        &mut tx,
        entry.id,
        acting.account_id,
        call.id,
        state.credits.cost_per_message_mc,
    )
    .await?;
    sqlx::query!(
        r#"
        UPDATE calls SET last_message_at = now(), message_count = message_count + 1
         WHERE id = $1
        "#,
        call.id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    super::wake_parties(state, &to);
    Ok(entry)
}

#[derive(Debug, Deserialize, Default)]
pub struct HistoryQuery {
    /// Entries after this `seq`.
    pub after: Option<i32>,
    pub limit: Option<i64>,
    /// Full-text search: any of the words (spec §8.4).
    pub q: Option<String>,
}

/// `GET /v1/calls/{id}/messages`: the stored transcript, oldest first; with
/// `q`, the matches, best first.
pub async fn history(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    q: HistoryQuery,
) -> Result<Vec<Value>, ApiError> {
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    access::can_read(state, user, &call).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let search = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let rows = sqlx::query_as!(
        Entry,
        r#"
        SELECT e.id, e.call_id, e.seq, e.kind, e.from_agent_id, e.text, e.data,
               e.blocked, e.safety, e.created_at, e.invocation_id,
               i.capability_name AS "invocation_capability?",
               i.status AS "invocation_status?"
          FROM call_entries e
          LEFT JOIN relay_invocations i ON i.id = e.invocation_id
         WHERE e.call_id = $1
           AND e.seq > $2
           AND ($3::text IS NULL OR e.search @@ calls_or_query($3))
         ORDER BY CASE WHEN $3::text IS NULL THEN 0
                       ELSE ts_rank_cd(e.search, calls_or_query($3)) END DESC,
                  e.seq ASC
         LIMIT $4
        "#,
        call.id,
        q.after.unwrap_or(0),
        search,
        limit,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows.iter().map(Entry::dto).collect())
}
