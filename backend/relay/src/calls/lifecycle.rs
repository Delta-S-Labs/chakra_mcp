//! Starting, answering and ending calls (spec §3), and deleting their
//! content early (spec §9).

use axum::http::StatusCode;
use chakramcp_shared::error::ApiError;
use chakramcp_shared::system_one::{note_or_refuse, NoteAbout};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::messages::{self, size_of};
use super::{
    access, append, bad_terms, busy, friendship, load_call, not_active, purge, push_unsupported,
    queue_message_charge, Call, NewEntry, Role, DEFAULT_IDLE_SECS, MAX_IDLE_SECS, MAX_REASON_CHARS,
    MAX_TOPIC_CHARS, MIN_IDLE_SECS,
};
use crate::auth::AuthUser;
use crate::events::record_audit;
use crate::state::RelayState;

#[derive(Debug, Deserialize)]
pub struct InviteRequest {
    pub from_agent_id: Uuid,
    pub to_agent_id: Uuid,
    /// `call` (the relay keeps memory) or `own` (the default).
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub ttl_secs: Option<i64>,
    #[serde(default)]
    pub idle_secs: Option<i64>,
    #[serde(default)]
    pub topic: Option<String>,
    /// An opening message's text.
    #[serde(default)]
    pub message: Option<String>,
}

struct Agent {
    id: Uuid,
    account_id: Uuid,
    mode: String,
    accept_calls: String,
}

/// `POST /v1/calls`: invite another agent to a call.
pub async fn invite(
    state: &RelayState,
    user: &AuthUser,
    req: InviteRequest,
) -> Result<Call, ApiError> {
    let cfg = state.calls;
    let memory = req.memory.as_deref().unwrap_or("own");
    if !matches!(memory, "call" | "own") {
        return Err(bad_terms("memory must be call or own"));
    }
    let ttl_secs = req.ttl_secs.unwrap_or(cfg.default_ttl_secs);
    if !(0..=cfg.max_ttl_secs).contains(&ttl_secs) {
        return Err(bad_terms(format!(
            "ttl_secs must be between 0 and {}",
            cfg.max_ttl_secs
        )));
    }
    let idle_secs = req.idle_secs.unwrap_or(DEFAULT_IDLE_SECS);
    if !(MIN_IDLE_SECS..=MAX_IDLE_SECS).contains(&idle_secs) {
        return Err(bad_terms(format!(
            "idle_secs must be between {MIN_IDLE_SECS} and {MAX_IDLE_SECS}"
        )));
    }
    let topic = req
        .topic
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    if topic
        .as_deref()
        .is_some_and(|t| t.chars().count() > MAX_TOPIC_CHARS)
    {
        return Err(bad_terms(format!(
            "a topic is at most {MAX_TOPIC_CHARS} characters"
        )));
    }
    if req.from_agent_id == req.to_agent_id {
        return Err(bad_terms("an agent can't call itself"));
    }
    let opening = req.message.filter(|m| !m.trim().is_empty());
    let opening_size = match &opening {
        Some(text) => size_of(text, None)?,
        None => 0,
    };

    let acting = access::act_as(state, user, req.from_agent_id).await?;
    let agents = sqlx::query_as!(
        Agent,
        r#"
        SELECT id, account_id, mode, accept_calls FROM agents
         WHERE id = ANY($1) AND tombstoned_at IS NULL
        "#,
        &[req.from_agent_id, req.to_agent_id],
    )
    .fetch_all(&state.db)
    .await?;
    let find = |id| agents.iter().find(|a| a.id == id);
    let (Some(caller), Some(callee)) = (find(req.from_agent_id), find(req.to_agent_id)) else {
        return Err(ApiError::NotFound);
    };
    if caller.mode == "push" || callee.mode == "push" {
        return Err(push_unsupported());
    }
    let friends = friendship(&state.db, caller.id, callee.id).await?;
    let allowed = match callee.accept_calls.as_str() {
        "nobody" => false,
        _ if caller.account_id == callee.account_id => true,
        "anyone" => true,
        _ => friends.is_some(),
    };
    if !allowed {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            "chk.calls.not_accepted",
            match callee.accept_calls.as_str() {
                "nobody" => "this agent doesn't take calls",
                _ => "this agent takes calls from its friends only",
            },
        ));
    }

    let load = sqlx::query!(
        r#"
        SELECT
          (SELECT count(*) FROM calls
            WHERE state = 'active' AND (caller_agent_id = $1 OR callee_agent_id = $1))
        + (SELECT count(*) FROM calls WHERE state = 'ringing' AND caller_agent_id = $1)
              AS "caller!",
          (SELECT count(*) FROM calls
            WHERE state = 'active' AND (caller_agent_id = $2 OR callee_agent_id = $2))
              AS "callee!"
        "#,
        caller.id,
        callee.id,
    )
    .fetch_one(&state.db)
    .await?;
    if load.caller >= cfg.max_active_per_agent {
        return Err(busy(format!(
            "this agent already has {} calls open",
            cfg.max_active_per_agent
        )));
    }
    if load.callee >= cfg.max_active_per_agent {
        return Err(busy("the agent you're calling is on too many calls"));
    }
    let limits = state.enforce_limits(acting.account_id, "calls").await;
    if let Some(outcome) = limits.refusal {
        return Err(state.refusal_error(outcome));
    }

    let topic = note_or_refuse(
        state.compliance.as_deref(),
        topic,
        NoteAbout {
            kind: "call_topic",
            capability_name: None,
            capability_description: None,
            proposal_message: None,
        },
    )
    .await?;
    let opening = match opening {
        Some(text) => {
            let s = messages::screen(
                state,
                &text,
                None,
                friends.as_ref().map(|f| f.relationship()),
            )
            .await?;
            // No call exists yet, so a refused opening refuses the invite.
            if let Some(block) = s.block {
                return Err(ApiError::Policy {
                    code: block.code,
                    message: block.reason,
                });
            }
            Some(s)
        }
        None => None,
    };

    // What call memory can do here, fixed for the call (spec §8.6).
    let tags = state.memory.is_some();
    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query!(
        r#"
        INSERT INTO calls
            (id, caller_agent_id, callee_agent_id, caller_account_id, callee_account_id,
             started_by_user_id, memory, memory_features, ttl_secs, idle_secs, topic, state)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'ringing')
        "#,
        id,
        caller.id,
        callee.id,
        caller.account_id,
        callee.account_id,
        user.user_id,
        memory,
        json!({ "tags": tags, "recall": tags }),
        ttl_secs as i32,
        idle_secs as i32,
        topic,
    )
    .execute(&mut *tx)
    .await;
    if let Err(sqlx::Error::Database(e)) = &inserted {
        if e.constraint() == Some("calls_ringing_pair") {
            return Err(busy("a call between these agents is already ringing"));
        }
    }
    inserted?;
    append(
        &mut tx,
        id,
        NewEntry::event("invite", Some(caller.id)),
        &[callee.id],
    )
    .await?;
    if let Some(s) = &opening {
        opening_message(
            &mut tx,
            state,
            id,
            caller.id,
            caller.account_id,
            callee.id,
            s,
            opening_size,
            memory == "call" && tags,
        )
        .await?;
    }
    tx.commit().await?;

    record_audit(
        &state.db,
        user,
        Some(caller.account_id),
        "call.started",
        "call",
        Some(id),
        Some(callee.id),
        "Started a call",
        json!({ "memory": memory, "ttl_secs": ttl_secs }),
    )
    .await;
    load_call(&state.db, id).await?.ok_or(ApiError::NotFound)
}

/// A first message riding on an invite or an accept: delivered, charged and
/// counted like any other.
#[allow(clippy::too_many_arguments)]
async fn opening_message(
    conn: &mut PgConnection,
    state: &RelayState,
    call_id: Uuid,
    from: Uuid,
    from_account: Uuid,
    to: Uuid,
    s: &messages::Screened,
    size: i32,
    tag: bool,
) -> Result<(), ApiError> {
    let entry = append(
        &mut *conn,
        call_id,
        NewEntry {
            kind: "message",
            from: Some(from),
            text: Some(&s.text),
            data: s.data.as_ref(),
            blocked: false,
            size_bytes: size,
            safety: s.findings.clone(),
            invocation_id: None,
            tag,
        },
        &[to],
    )
    .await?;
    queue_message_charge(
        &mut *conn,
        entry.id,
        from_account,
        call_id,
        state.credits.cost_per_message_mc,
    )
    .await?;
    sqlx::query!(
        r#"
        UPDATE calls SET last_message_at = now(), message_count = message_count + 1
         WHERE id = $1
        "#,
        call_id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct AcceptRequest {
    pub agent_id: Uuid,
    /// A first reply's text.
    #[serde(default)]
    pub message: Option<String>,
}

/// The call `agent_id` may answer: it's the callee and the call is ringing.
async fn ringing_for(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    agent_id: Uuid,
) -> Result<(Call, access::Acting), ApiError> {
    let acting = access::act_as(state, user, agent_id).await?;
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    match call.role_of(agent_id) {
        Some(Role::Callee) => {}
        Some(Role::Caller) => return Err(ApiError::Forbidden),
        None => return Err(ApiError::NotFound),
    }
    if call.state != "ringing" {
        return Err(not_active(format!("this call is {}", call.state)));
    }
    Ok((call, acting))
}

/// `POST /v1/calls/{id}/accept`.
pub async fn accept(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    req: AcceptRequest,
) -> Result<Call, ApiError> {
    let (call, acting) = ringing_for(state, user, call_id, req.agent_id).await?;
    let caller = call.caller.agent_id.ok_or(ApiError::NotFound)?;
    let active = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "n!" FROM calls
         WHERE state = 'active' AND (caller_agent_id = $1 OR callee_agent_id = $1)
        "#,
        acting.agent_id,
    )
    .fetch_one(&state.db)
    .await?;
    if active >= state.calls.max_active_per_agent {
        return Err(busy(format!(
            "this agent already has {} calls active",
            state.calls.max_active_per_agent
        )));
    }
    let reply = req.message.filter(|m| !m.trim().is_empty());
    let reply_size = match &reply {
        Some(text) => size_of(text, None)?,
        None => 0,
    };
    let limits = state.enforce_limits(acting.account_id, "calls").await;
    if let Some(outcome) = limits.refusal {
        return Err(state.refusal_error(outcome));
    }
    let reply = match reply {
        Some(text) => {
            let friends = messages::relationship_of(state, &call).await?;
            let s = messages::screen(
                state,
                &text,
                None,
                friends.as_ref().map(|f| f.relationship()),
            )
            .await?;
            // A refused first reply refuses the accept: the call keeps ringing.
            if let Some(block) = s.block {
                return Err(ApiError::Policy {
                    code: block.code,
                    message: block.reason,
                });
            }
            Some(s)
        }
        None => None,
    };

    let mut tx = state.db.begin().await?;
    let answered = sqlx::query_scalar!(
        r#"
        UPDATE calls SET state = 'active', answered_at = now(), last_message_at = now()
         WHERE id = $1 AND state = 'ringing'
        RETURNING id
        "#,
        call.id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if answered.is_none() {
        return Err(not_active("this call is no longer ringing"));
    }
    append(
        &mut tx,
        call.id,
        NewEntry::event("accepted", Some(acting.agent_id)),
        &[caller],
    )
    .await?;
    if let Some(s) = &reply {
        opening_message(
            &mut tx,
            state,
            call.id,
            acting.agent_id,
            acting.account_id,
            caller,
            s,
            reply_size,
            call.tags_on(),
        )
        .await?;
    }
    tx.commit().await?;
    audit(
        state,
        user,
        acting.account_id,
        "call.answered",
        &call,
        "Accepted a call",
        json!({ "answer": "accepted" }),
    )
    .await;
    load_call(&state.db, call.id)
        .await?
        .ok_or(ApiError::NotFound)
}

#[derive(Debug, Deserialize)]
pub struct DeclineRequest {
    pub agent_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /v1/calls/{id}/decline`.
pub async fn decline(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    req: DeclineRequest,
) -> Result<Call, ApiError> {
    let (call, acting) = ringing_for(state, user, call_id, req.agent_id).await?;
    let reason = req
        .reason
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty());
    if reason
        .as_deref()
        .is_some_and(|r| r.chars().count() > MAX_REASON_CHARS)
    {
        return Err(bad_terms(format!(
            "a reason is at most {MAX_REASON_CHARS} characters"
        )));
    }
    let reason = note_or_refuse(
        state.compliance.as_deref(),
        reason,
        NoteAbout {
            kind: "call_decline_reason",
            capability_name: None,
            capability_description: None,
            proposal_message: None,
        },
    )
    .await?;

    let mut tx = state.db.begin().await?;
    let declined = sqlx::query_scalar!(
        r#"
        UPDATE calls
           SET state = 'declined', ended_at = now(), decline_reason = $2,
               purge_at = now() + make_interval(secs => ttl_secs)
         WHERE id = $1 AND state = 'ringing'
        RETURNING id
        "#,
        call.id,
        reason,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if declined.is_none() {
        return Err(not_active("this call is no longer ringing"));
    }
    let to: Vec<Uuid> = call.caller.agent_id.into_iter().collect();
    let mut entry = NewEntry::event("declined", Some(acting.agent_id));
    entry.text = reason.as_deref();
    append(&mut tx, call.id, entry, &to).await?;
    tx.commit().await?;
    audit(
        state,
        user,
        acting.account_id,
        "call.answered",
        &call,
        "Declined a call",
        json!({ "answer": "declined" }),
    )
    .await;
    load_call(&state.db, call.id)
        .await?
        .ok_or(ApiError::NotFound)
}

#[derive(Debug, Deserialize)]
pub struct EndRequest {
    pub agent_id: Uuid,
}

/// `POST /v1/calls/{id}/end`: a party hangs up (or withdraws a ringing
/// invite). The other party is told.
pub async fn end(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    req: EndRequest,
) -> Result<Call, ApiError> {
    let acting = access::act_as(state, user, req.agent_id).await?;
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let role = call.role_of(acting.agent_id).ok_or(ApiError::NotFound)?;
    let other = call.party(role.other());
    let to: Vec<Uuid> = other.agent_id.filter(|_| !other.gone).into_iter().collect();
    let mut tx = state.db.begin().await?;
    end_in(&mut tx, &call, role.as_str(), Some(acting.agent_id), &to).await?;
    tx.commit().await?;
    audit(
        state,
        user,
        acting.account_id,
        "call.ended",
        &call,
        "Ended a call",
        json!({ "ended_by": role.as_str() }),
    )
    .await;
    load_call(&state.db, call.id)
        .await?
        .ok_or(ApiError::NotFound)
}

/// End an open call in `conn`'s transaction, with its `ended` entry
/// delivered to `to`. 409 when it's already final.
async fn end_in(
    conn: &mut PgConnection,
    call: &Call,
    ended_by: &str,
    from: Option<Uuid>,
    to: &[Uuid],
) -> Result<(), ApiError> {
    let ended = sqlx::query_scalar!(
        r#"
        UPDATE calls
           SET state = 'ended', ended_at = now(), ended_by = $2,
               purge_at = now() + make_interval(secs => ttl_secs)
         WHERE id = $1 AND state IN ('ringing', 'active')
        RETURNING id
        "#,
        call.id,
        ended_by,
    )
    .fetch_optional(&mut *conn)
    .await?;
    if ended.is_none() {
        return Err(not_active(format!("this call is already {}", call.state)));
    }
    append(conn, call.id, NewEntry::event("ended", from), to).await?;
    Ok(())
}

/// `POST /v1/calls/{id}/owner-end`: an owner ends a call from the web. Both
/// agents are told.
pub async fn owner_end(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
) -> Result<Call, ApiError> {
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    access::owner(state, user, &call).await?;
    let mut tx = state.db.begin().await?;
    end_in(&mut tx, &call, "owner", None, &call.live_agents()).await?;
    tx.commit().await?;
    let account = owner_account(state, user, &call).await?;
    audit(
        state,
        user,
        account,
        "call.ended",
        &call,
        "Ended a call",
        json!({ "ended_by": "owner" }),
    )
    .await;
    load_call(&state.db, call.id)
        .await?
        .ok_or(ApiError::NotFound)
}

/// `DELETE /v1/calls/{id}/content`: an owner deletes a call's content now,
/// for both sides (spec §9). An open call is ended first.
pub async fn delete_content(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
) -> Result<(), ApiError> {
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    access::owner(state, user, &call).await?;
    let mut tx = state.db.begin().await?;
    if call.is_open() {
        end_in(&mut tx, &call, "owner", None, &call.live_agents()).await?;
    }
    purge::close_invocations(&mut tx, call.id).await?;
    purge::purge_call(&mut tx, call.id).await?;
    tx.commit().await?;
    let account = owner_account(state, user, &call).await?;
    audit(
        state,
        user,
        account,
        "call.content_deleted",
        &call,
        "Deleted a call's content",
        json!({}),
    )
    .await;
    Ok(())
}

/// The party account the owner acting on `call` belongs to, for the audit
/// trail.
async fn owner_account(state: &RelayState, user: &AuthUser, call: &Call) -> Result<Uuid, ApiError> {
    if crate::auth::user_is_member(&state.db, user.user_id, call.caller.account_id).await? {
        Ok(call.caller.account_id)
    } else {
        Ok(call.callee.account_id)
    }
}

async fn audit(
    state: &RelayState,
    user: &AuthUser,
    account_id: Uuid,
    action: &str,
    call: &Call,
    summary: &str,
    metadata: serde_json::Value,
) {
    record_audit(
        &state.db,
        user,
        Some(account_id),
        action,
        "call",
        Some(call.id),
        None,
        summary,
        metadata,
    )
    .await;
}
