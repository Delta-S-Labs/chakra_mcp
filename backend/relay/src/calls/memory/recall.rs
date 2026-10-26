//! Recall (spec §8.4): `POST /v1/calls/{id}/memory/ask`. An agent asks the
//! call's memory a question; Jev picks the messages that answer it from a
//! shortlist, and the agent gets their full text, to answer for itself.

use std::collections::HashSet;

use axum::http::StatusCode;
use chakramcp_shared::calls_memory::{
    read_recall, recall_request, Candidate, MAX_CANDIDATES, SNIPPET_CHARS,
};
use chakramcp_shared::error::ApiError;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{load_bundle, meter, side_of, unavailable, Stored};
use crate::auth::AuthUser;
use crate::calls::{access, bad_terms, load_call, not_active};
use crate::state::RelayState;

/// Recalls one agent may make on one call per hour.
pub const PER_HOUR: i64 = 30;
/// The longest question.
const MAX_QUESTION_BYTES: usize = 1024;
/// Full-text hits in the shortlist.
const SEARCH_HITS: i64 = 48;

#[derive(Debug, Deserialize)]
pub struct AskRequest {
    pub agent_id: Uuid,
    pub question: String,
}

pub async fn ask(
    state: &RelayState,
    user: &AuthUser,
    call_id: Uuid,
    req: AskRequest,
) -> Result<Value, ApiError> {
    let acting = access::act_as(state, user, req.agent_id).await?;
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    call.role_of(acting.agent_id).ok_or(ApiError::NotFound)?;
    let Some(client) = state.memory.clone() else {
        return Err(unavailable("call memory is off on this relay"));
    };
    if call.memory != "call" || call.memory_features["recall"] != true {
        return Err(unavailable(
            "this call's memory can't be asked (memory own, or off when the call was placed)",
        ));
    }
    if call.purged_at.is_some() {
        return Err(not_active("this call's content has been deleted"));
    }
    let question = req.question.trim();
    if question.is_empty() || question.len() > MAX_QUESTION_BYTES {
        return Err(bad_terms(format!(
            "a question is 1 to {MAX_QUESTION_BYTES} bytes"
        )));
    }
    let recent = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "n!" FROM call_memory_requests
         WHERE call_id = $1 AND agent_id = $2 AND kind = 'recall'
           AND created_at > now() - interval '1 hour'
        "#,
        call.id,
        acting.agent_id,
    )
    .fetch_one(&state.db)
    .await?;
    if recent >= PER_HOUR {
        return Err(ApiError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            "chk.calls.slow_down",
            format!("at most {PER_HOUR} questions an hour to one call's memory"),
        ));
    }
    // The caller pays for memory, so its account's limits apply.
    let limits = state
        .enforce_limits(call.caller.account_id, "calls_memory")
        .await;
    if let Some(outcome) = limits.refusal {
        return Err(state.refusal_error(outcome));
    }

    // The shortlist: full-text hits first, then what the bundle holds.
    let hits = sqlx::query!(
        r#"
        SELECT seq, from_agent_id,
               ts_headline('simple', text, calls_or_query($2),
                           'MaxFragments=1, MaxWords=45, MinWords=15, StartSel="", StopSel=""')
                   AS "snippet!"
          FROM call_entries
         WHERE call_id = $1 AND kind = 'message' AND NOT blocked AND text IS NOT NULL
           AND search @@ calls_or_query($2)
         ORDER BY ts_rank_cd(search, calls_or_query($2)) DESC, seq DESC
         LIMIT $3
        "#,
        call.id,
        question,
        SEARCH_HITS,
    )
    .fetch_all(&state.db)
    .await?;
    let mut seen = HashSet::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for h in hits {
        if seen.insert(h.seq) {
            candidates.push(Candidate {
                seq: h.seq,
                from: side_of(&call, h.from_agent_id),
                snippet: h.snippet,
            });
        }
    }
    let bundle = load_bundle(state, &call).await?;
    for part in ["pinned", "recent"] {
        for item in bundle[part].as_array().into_iter().flatten() {
            let (Some(seq), Some(text)) = (item["seq"].as_i64(), item["text"].as_str()) else {
                continue;
            };
            let seq = seq as i32;
            if seen.insert(seq) {
                let from = item["from"].as_str().and_then(|f| f.parse().ok());
                candidates.push(Candidate {
                    seq,
                    from: side_of(&call, from),
                    snippet: text.chars().take(SNIPPET_CHARS).collect(),
                });
            }
        }
    }
    candidates.truncate(MAX_CANDIDATES);
    if candidates.is_empty() {
        return Ok(json!({ "answered": null, "messages": [] }));
    }

    let (body_state, questions) = recall_request(question, &candidates);
    let asked = client
        .ask_questions(&body_state, &questions)
        .await
        .map_err(|e| {
            tracing::warn!(call_id = %call.id, error = %e, "calls memory: recall failed");
            unavailable("the call's memory couldn't be asked just now: try again")
        })?;
    let (answered, ranked) = read_recall(&asked.answers);

    let mut tx = state.db.begin().await?;
    meter(
        &mut tx,
        state,
        &call,
        Some(acting.agent_id),
        "recall",
        asked.input_tokens,
    )
    .await?;
    tx.commit().await?;

    let seqs: Vec<i32> = ranked.iter().map(|(s, _)| *s).collect();
    let stored = sqlx::query_as!(
        Stored,
        r#"
        SELECT seq, from_agent_id, text, created_at FROM call_entries
         WHERE call_id = $1 AND seq = ANY($2)
        "#,
        call.id,
        &seqs,
    )
    .fetch_all(&state.db)
    .await?;
    let messages: Vec<Value> = ranked
        .iter()
        .filter_map(|(seq, p)| {
            let s = stored.iter().find(|s| s.seq == *seq)?;
            Some(json!({
                "seq": s.seq,
                "from": s.from_agent_id,
                "text": s.text,
                "at": s.created_at,
                "p": (p * 100.0).round() / 100.0,
            }))
        })
        .collect();
    Ok(json!({ "answered": answered, "messages": messages }))
}
