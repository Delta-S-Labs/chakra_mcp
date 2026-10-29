//! Posting a result (spec §5): one function for REST
//! `POST /v1/invocations/{id}/result` and MCP `respond`.
//!
//! For a `human_in_loop` capability, a reply is a draft until an approver
//! confirms it on the web; `confirmed_by_human` from the caller is ignored.
//! An approver answering from the web app (an interactive session) is
//! confirmed at once. Other capabilities finish as they always have.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use chakramcp_shared::consent::decide::Actor;
use chakramcp_shared::consent::reply::{self, Draft, ReplyTarget};
use chakramcp_shared::error::ApiError;
use chakramcp_shared::safety::{Channel, Safety};

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
        SELECT i.status, i.created_at, i.capability_id,
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

    if row.status != "in_progress" {
        return Err(ApiError::Conflict(format!(
            "invocation is {}; only in_progress can be completed",
            row.status
        )));
    }
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

    // Message safety (when on): screen the reply before anything below
    // stores or releases it, so drafts, confirmations and tokens all cover
    // what the caller will see.
    let (mut output, mut error) = (output, error);
    let mut safety = None;
    if let Some(checker) = state.compliance.as_deref().filter(|c| c.safety_enabled()) {
        let mut sent = serde_json::json!({ "status": status });
        if let Some(o) = &output {
            sent["output"] = o.clone();
        }
        if let Some(e) = &error {
            sent["error"] = serde_json::json!(e);
        }
        let subject = crate::compliance::Subject {
            capability_name: row.capability_name.as_deref().unwrap_or("(deleted)"),
            capability_description: row
                .capability_description
                .as_deref()
                .filter(|d| !d.trim().is_empty()),
            grant_purpose: None,
            relationship: None,
            input: &sent,
        };
        let screened = checker
            .screen(
                Channel::Reply,
                &subject,
                "",
                row.capability_output_schema.as_ref(),
                "/output",
            )
            .await
            .map_err(|_| crate::handlers::invoke::too_large_to_screen())?;
        store_reply_report(&state.db, id, &screened.report).await?;
        if let Some(block) = screened.block {
            return Err(ApiError::Policy {
                code: block.code,
                message: block.reason,
            });
        }
        if output.is_some() {
            output = screened.value.get("output").cloned();
        }
        if error.is_some() {
            error = screened.value["error"].as_str().map(str::to_owned);
        }
        safety = (!screened.findings.is_empty()).then_some(Safety {
            request: None,
            reply: Some(screened.findings),
        });
    }

    if row.capability_semantics.as_deref() == Some("human_in_loop") {
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
        let draft = Draft {
            status: status.to_owned(),
            output,
            error,
        };
        let approvers = row.approvers.unwrap_or_default();
        let may_decide = crate::auth::is_interactive(state, user).await?
            && chakramcp_shared::consent::approvers::may_decide(
                &state.db,
                user.user_id,
                granter_account,
                &approvers,
            )
            .await?;
        if may_decide {
            let actor = Actor {
                user_id: user.user_id,
                minted_jti: user.minted_jti,
            };
            let elapsed_ms =
                reply::confirm_now(&state.db, &state.wake, actor, &target, &draft).await?;
            return Ok(Submitted::Finished {
                status: draft.status,
                elapsed_ms,
                safety,
            });
        }
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
    let output = match preview {
        Preview::Truncate => output
            .as_ref()
            .map(crate::handlers::invoke::truncate_for_audit),
        Preview::Whole => output,
    };
    let finished = sqlx::query!(
        r#"
        UPDATE relay_invocations
        SET status = $2,
            elapsed_ms = $3,
            error_message = $4,
            output_preview = $5
        WHERE id = $1 AND status = 'in_progress'
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

/// Keep a reply's screening report (message safety §5), drafts and blocked
/// replies included: `trust_snapshot.system_one.reply_safety`.
async fn store_reply_report(db: &sqlx::PgPool, id: Uuid, report: &Value) -> Result<(), ApiError> {
    sqlx::query!(
        r#"
        UPDATE relay_invocations
           SET trust_snapshot = COALESCE(trust_snapshot, '{}'::jsonb)
               || jsonb_build_object(
                    'system_one',
                    COALESCE(trust_snapshot->'system_one', '{}'::jsonb)
                        || jsonb_build_object('reply_safety', $2::jsonb))
         WHERE id = $1
        "#,
        id,
        report,
    )
    .execute(db)
    .await?;
    Ok(())
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
