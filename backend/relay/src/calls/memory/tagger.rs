//! Tagging (spec §8.2): a background worker asks Jev about new messages in
//! calls whose memory has tags, in batches that fit one request.
//!
//! Entries are claimed with a lease, not a lock: a claim stamps `tagged_at`
//! and counts the attempt in `tags`, so a replica skips what another claimed
//! in the last 5 minutes, and a crash only delays. Holding the call's row
//! lock through a Jev request would make sends wait for it. Three failed
//! attempts mark an entry `error`; memory then does without its tags.

use std::time::Duration;

use chakramcp_shared::calls_memory::{fit_batch, read_tags, tag_request, Message, Open};
use chakramcp_shared::error::ApiError;
use uuid::Uuid;

use super::{meter, side_of};
use crate::calls::load_call;
use crate::state::RelayState;

/// How often the tagger runs.
pub const INTERVAL: Duration = Duration::from_secs(15);
/// Calls looked at per pass, and messages claimed per call.
const CALLS_PER_PASS: i64 = 20;
const BATCH: i64 = 16;
/// Attempts before an entry is given up on.
const MAX_ATTEMPTS: i32 = 3;

/// Start the tagger. Call once per process, with calls and memory on.
pub fn spawn_tagger(state: RelayState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match tag_once(&state).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(tagged = n, "calls memory tagger"),
                Err(e) => tracing::warn!(error = %e, "calls memory tagger failed; retrying"),
            }
        }
    });
}

/// One pass: up to [`CALLS_PER_PASS`] calls, one batch each. Returns how many
/// messages were tagged.
pub async fn tag_once(state: &RelayState) -> Result<u64, ApiError> {
    let Some(client) = state.memory.clone() else {
        return Ok(0);
    };
    let due: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT DISTINCT e.call_id AS "call_id!"
          FROM call_entries e
          JOIN calls c ON c.id = e.call_id
         WHERE e.tag_state = 'pending'
           AND (e.tagged_at IS NULL OR e.tagged_at < now() - interval '5 minutes')
           AND c.purged_at IS NULL
         LIMIT $1
        "#,
        CALLS_PER_PASS,
    )
    .fetch_all(&state.db)
    .await?;

    let mut tagged = 0;
    for call_id in due {
        let Some(call) = load_call(&state.db, call_id).await? else {
            continue;
        };
        // Memory is the caller's to pay for: a caller out of credits waits.
        if state.credits.enabled && state.credit_cache.is_blocked(call.caller.account_id) {
            continue;
        }
        let claimed = sqlx::query!(
            r#"
            UPDATE call_entries
               SET tagged_at = now(),
                   tags = jsonb_build_object(
                       'attempts', COALESCE((tags->>'attempts')::int, 0) + 1)
             WHERE id IN (
                   SELECT id FROM call_entries
                    WHERE call_id = $1 AND tag_state = 'pending'
                      AND (tagged_at IS NULL OR tagged_at < now() - interval '5 minutes')
                    ORDER BY seq
                    LIMIT $2
                      FOR UPDATE SKIP LOCKED)
            RETURNING id, seq, from_agent_id, text,
                      (tags->>'attempts')::int AS "attempts!"
            "#,
            call.id,
            BATCH,
        )
        .fetch_all(&state.db)
        .await?;
        if claimed.is_empty() {
            continue;
        }
        let mut claimed = claimed;
        claimed.sort_by_key(|c| c.seq);

        let mut batch = Vec::with_capacity(claimed.len());
        for c in &claimed {
            let previous = sqlx::query_scalar!(
                r#"
                SELECT text AS "text!" FROM call_entries
                 WHERE call_id = $1 AND kind = 'message' AND seq < $2 AND text IS NOT NULL
                 ORDER BY seq DESC LIMIT 1
                "#,
                call.id,
                c.seq,
            )
            .fetch_optional(&state.db)
            .await?;
            batch.push(Message {
                seq: c.seq,
                from: side_of(&call, c.from_agent_id),
                text: c.text.clone().unwrap_or_default(),
                previous,
            });
        }
        let open = open_questions(state, call.id).await?;
        let n = fit_batch(call.topic.as_deref(), &batch, &open);
        // What didn't fit goes back for the next pass, its attempt uncounted.
        let unfit: Vec<Uuid> = claimed[n..].iter().map(|c| c.id).collect();
        if !unfit.is_empty() {
            sqlx::query!(
                r#"
                UPDATE call_entries
                   SET tagged_at = NULL,
                       tags = jsonb_build_object('attempts', (tags->>'attempts')::int - 1)
                 WHERE id = ANY($1)
                "#,
                &unfit,
            )
            .execute(&state.db)
            .await?;
        }
        let batch = &batch[..n];

        let (body_state, questions) = tag_request(call.topic.as_deref(), batch, &open);
        match client.ask_questions(&body_state, &questions).await {
            Ok(asked) => {
                let mut tx = state.db.begin().await?;
                for (seq, tags) in read_tags(&asked.answers, batch) {
                    sqlx::query!(
                        r#"
                        UPDATE call_entries
                           SET tags = $3, tag_state = 'done', tagged_at = now()
                         WHERE call_id = $1 AND seq = $2
                        "#,
                        call.id,
                        seq,
                        serde_json::to_value(&tags).map_err(|e| ApiError::Internal(e.into()))?,
                    )
                    .execute(&mut *tx)
                    .await?;
                }
                meter(&mut tx, state, &call, None, "tag", asked.input_tokens).await?;
                tx.commit().await?;
                tagged += n as u64;
            }
            Err(error) => {
                tracing::warn!(call_id = %call.id, %error, "calls memory: tagging failed");
                let spent: Vec<Uuid> = claimed[..n]
                    .iter()
                    .filter(|c| c.attempts >= MAX_ATTEMPTS)
                    .map(|c| c.id)
                    .collect();
                if !spent.is_empty() {
                    sqlx::query!(
                        "UPDATE call_entries SET tag_state = 'error' WHERE id = ANY($1)",
                        &spent,
                    )
                    .execute(&state.db)
                    .await?;
                }
            }
        }
    }
    Ok(tagged)
}

/// The call's questions not yet answered: tagged `open_question`, with no
/// later message tagged as answering them. The newest first.
async fn open_questions(state: &RelayState, call_id: Uuid) -> Result<Vec<Open>, ApiError> {
    let threshold = state.calls.tag_threshold;
    let rows = sqlx::query!(
        r#"
        SELECT q.seq, q.text AS "text!"
          FROM call_entries q
         WHERE q.call_id = $1 AND q.tag_state = 'done' AND q.text IS NOT NULL
           AND (q.tags->>'open_question')::float8 >= $2
           AND NOT EXISTS (
               SELECT 1 FROM call_entries a
                WHERE a.call_id = q.call_id AND a.tag_state = 'done'
                  AND (a.tags->>'resolves')::int = q.seq
                  AND (a.tags->>'resolves_p')::float8 >= $2)
         ORDER BY q.seq DESC
         LIMIT $3
        "#,
        call_id,
        threshold,
        chakramcp_shared::calls_memory::MAX_OPEN as i64,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Open {
            seq: r.seq,
            text: r.text,
        })
        .collect())
}
