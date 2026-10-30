//! Posting a result (spec §5): one function for REST
//! `POST /v1/invocations/{id}/result` and MCP `respond`.
//!
//! For a `human_in_loop` capability, a reply is a draft until an approver
//! confirms it on the web; `confirmed_by_human` from the caller is ignored.
//! An approver answering from the web app (an interactive session) is
//! confirmed at once, whether or not the agent had claimed the call
//! (`shared::consent::reply::answer_as_owner`). Other capabilities finish
//! as they always have.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use chakramcp_shared::consent::decide::Actor;
pub use chakramcp_shared::consent::reply::already_settled;
use chakramcp_shared::consent::reply::{self, Draft, ReplyAbout, ReplyTarget};
use chakramcp_shared::error::ApiError;
use chakramcp_shared::safety::Safety;

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

/// How a surface stores an autonomous result's output: REST keeps a
/// 16 KiB preview, MCP the whole value (as each always has).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    Truncate,
    Whole,
}

#[derive(Debug)]
pub enum Submitted {
    /// The invocation finished with this status.
    Finished {
        status: String,
        elapsed_ms: i32,
        /// What message safety removed or flagged in the reply.
        safety: Option<Safety>,
    },
    /// Stored as a draft for an approver to confirm.
    Drafted {
        consent_request_id: Uuid,
        approve_url: String,
        expires_at: DateTime<Utc>,
        safety: Option<Safety>,
    },
}

pub async fn submit_result(
    state: &RelayState,
    user: &AuthUser,
    id: Uuid,
    status: &str,
    output: Option<Value>,
    error: Option<String>,
    preview: Preview,
) -> Result<Submitted, ApiError> {
    if !matches!(status, "succeeded" | "failed") {
        return Err(ApiError::InvalidRequest(
            "status must be 'succeeded' or 'failed'".into(),
        ));
    }
    let row = sqlx::query!(
        r#"
        SELECT i.status, i.deliveries, i.created_at, i.capability_id,
               i.granter_agent_id, i.grantee_agent_id,
               ga.account_id AS "granter_account_id?",
               ea.account_id AS "grantee_account_id?",
               c.semantics AS "capability_semantics?",
               c.name AS "capability_name?",
               c.description AS "capability_description?",
               c.output_schema AS "capability_output_schema?",
               c.consent_approvers AS "approvers?",
               c.consent_wait_secs AS "wait_secs?"
          FROM relay_invocations i
          LEFT JOIN agents ga ON ga.id = i.granter_agent_id
          LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
          LEFT JOIN agent_capabilities c ON c.id = i.capability_id
         WHERE i.id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Auth: caller must be a member of the granter's account. The agent
    // could be deleted in the meantime; if so, no one can report a result.
    let granter_account = row.granter_account_id.ok_or(ApiError::Forbidden)?;
    if !user_is_member(&state.db, user.user_id, granter_account).await? {
        return Err(ApiError::Forbidden);
    }
    // Answering as the called agent needs the credential's scope too (MCP and
    // discovery spec §4.2).
    if let Some(agent) = row.granter_agent_id {
        crate::auth::require_scope(state, user, agent).await?;
    }
    let draft = Draft {
        status: status.to_owned(),
        output,
        error,
    };

    // One of the owner's approvers, signed in to the web, answers a
    // `human_in_loop` call themselves: released at once, claimed or not
    // (waking pull agents §9). Checked before screening, so a reply from
    // anyone else becomes a draft below and is screened once.
    let human_in_loop = row.capability_semantics.as_deref() == Some("human_in_loop");
    if human_in_loop
        && crate::auth::is_interactive(state, user).await?
        && chakramcp_shared::consent::approvers::may_decide(
            &state.db,
            user.user_id,
            granter_account,
            row.approvers.as_deref().unwrap_or_default(),
        )
        .await?
    {
        let answered = reply::answer_as_owner(
            &state.db,
            &state.wake,
            state.compliance.as_deref(),
            Actor {
                user_id: user.user_id,
                minted_jti: user.minted_jti,
            },
            id,
            draft,
        )
        .await?;
        return Ok(Submitted::Finished {
            status: answered.status,
            elapsed_ms: answered.elapsed_ms,
            safety: answered.safety,
        });
    }

    check_answerable(&row.status, row.deliveries, human_in_loop)?;
    // Message safety (when on): screen the reply before anything below
    // stores or releases it, so drafts, confirmations and tokens all cover
    // what the caller will see.
    let about = ReplyAbout {
        capability_name: row.capability_name.as_deref().unwrap_or("(deleted)"),
        capability_description: row.capability_description.as_deref(),
        output_schema: row.capability_output_schema.as_ref(),
    };
    let (draft, safety) =
        reply::screen_reply(&state.db, state.compliance.as_deref(), id, &about, draft).await?;

    if human_in_loop {
        let (Some(capability_id), Some(granter), Some(grantee), Some(grantee_account)) = (
            row.capability_id,
            row.granter_agent_id,
            row.grantee_agent_id,
            row.grantee_account_id,
        ) else {
            return Err(ApiError::Conflict(
                "the invocation's agents or capability no longer exist".into(),
            ));
        };
        let target = ReplyTarget {
            invocation_id: id,
            capability_id,
            granter_agent_id: granter,
            grantee_agent_id: grantee,
            grantee_account_id: grantee_account,
            wait_secs: row.wait_secs.unwrap_or(86_400),
        };
        let stored = reply::store_draft(&state.db, &target, &draft).await?;
        return Ok(Submitted::Drafted {
            consent_request_id: stored.consent_request_id,
            approve_url: super::notices::approve_url(state, stored.consent_request_id),
            expires_at: stored.expires_at,
            safety,
        });
    }

    // Wall time from enqueue to result.
    let elapsed_ms = (Utc::now() - row.created_at)
        .num_milliseconds()
        .clamp(0, i32::MAX as i64) as i32;
    let Draft { output, error, .. } = draft;
    let output = match preview {
        Preview::Truncate => output
            .as_ref()
            .map(crate::handlers::invoke::truncate_for_audit),
        Preview::Whole => output,
    };
    // The first answer wins: a late one lands on a call whose lease ran
    // out, until someone else answers it or its deadline passes.
    let finished = sqlx::query!(
        r#"
        UPDATE relay_invocations
        SET status = $2,
            elapsed_ms = $3,
            error_message = $4,
            output_preview = $5,
            lease_until = NULL
        WHERE id = $1
          AND (status = 'in_progress' OR (status = 'pending' AND deliveries > 0))
        "#,
        id,
        status,
        elapsed_ms,
        error.as_deref(),
        output.unwrap_or(Value::Null),
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    if finished == 0 {
        return Err(already_settled());
    }
    crate::telemetry::record_invocation_outcome(
        crate::telemetry::Mode::Pull,
        status,
        i64::from(elapsed_ms),
        finished,
    );
    Ok(Submitted::Finished {
        status: status.to_owned(),
        elapsed_ms,
        safety,
    })
}

/// Whether a call in `status`, claimed `deliveries` times, takes a result
/// (waking pull agents §9): a claimed one, or one claimed before whose lease
/// ran out (a late answer). A `human_in_loop` reply needs the claim, since
/// it becomes a draft for the owner.
fn check_answerable(status: &str, deliveries: i32, human_in_loop: bool) -> Result<(), ApiError> {
    match status {
        "in_progress" => Ok(()),
        "pending" if deliveries > 0 && !human_in_loop => Ok(()),
        "pending" => Err(ApiError::coded(
            axum::http::StatusCode::CONFLICT,
            "chk.invocation.not_claimed",
            "this invocation hasn't been claimed: pull it from the inbox first",
        )),
        "succeeded" | "failed" | "rejected" | "timeout" => Err(already_settled()),
        other => Err(ApiError::Conflict(format!(
            "invocation is {other}; only in_progress can be completed"
        ))),
    }
}

impl Submitted {
    /// The `202` body for a draft (REST and MCP alike).
    pub fn draft_body(&self) -> Option<Value> {
        match self {
            Submitted::Drafted {
                consent_request_id,
                approve_url,
                expires_at,
                safety,
            } => {
                let mut body = serde_json::json!({
                    "status": "awaiting_confirmation",
                    "consent_request_id": consent_request_id,
                    "approve_url": approve_url,
                    "expires_at": expires_at,
                });
                if let Some(safety) = safety {
                    body["safety"] = serde_json::json!(safety);
                }
                Some(body)
            }
            Submitted::Finished { .. } => None,
        }
    }
}
