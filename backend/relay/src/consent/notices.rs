//! Agent notices (spec §4.2, §5): what an agent is told out of band of its
//! inbox.
//!
//! - `consent_requested`: a request to the agent waits for its owner's
//!   approval; the agent passes the link on to its human.
//! - `reply_confirmed`, `reply_rejected`, `reply_expired`: what became of a
//!   reply it drafted for a `human_in_loop` capability.
//!
//! An agent claims its undelivered notices, oldest first, like its inbox
//! (`GET /v1/inbox/notices`, MCP `pull_notices`); each is delivered once.
//! A notice carries ids, names and terms, never the held input.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::state::RelayState;

pub const DEFAULT_LIMIT: i64 = 25;
pub const MAX_LIMIT: i64 = 100;

#[derive(Debug, Serialize)]
pub struct Notice {
    pub id: Uuid,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    /// The consent request the notice is about: for `consent_requested`,
    /// the one waiting for approval; for `reply_*`, the reply's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_request: Option<ConsentRequestNotice>,
}

#[derive(Debug, Serialize)]
pub struct ConsentRequestNotice {
    pub id: Uuid,
    pub invocation_id: Uuid,
    pub status: String,
    pub capability_id: Uuid,
    pub capability_name: Option<String>,
    pub requester: NoticeRequester,
    pub terms: Value,
    pub expires_at: DateTime<Utc>,
    /// Where the owner approves or denies it, signed in.
    pub approve_url: String,
}

#[derive(Debug, Serialize)]
pub struct NoticeRequester {
    pub agent_id: Uuid,
    pub agent_display_name: Option<String>,
    pub account_id: Uuid,
    pub account_display_name: Option<String>,
    pub verified: bool,
}

/// The approval page for request `id`: the web app on chakramcp.com, the
/// server's built-in page when self-hosted (its frontend URL is itself).
pub fn approve_url(state: &RelayState, id: Uuid) -> String {
    format!(
        "{}/app/approvals/{id}",
        state.config.frontend_base_url.trim_end_matches('/')
    )
}

/// Claim up to `limit` undelivered notices for `agent_id`, oldest first,
/// waiting up to `wait` seconds when there are none (waking pull agents
/// §6). Returns them with the seconds honoured. The caller must be a member
/// of the agent's account and allowed to manage it, as for its inbox.
pub async fn claim(
    state: &RelayState,
    user: &AuthUser,
    agent_id: Uuid,
    limit: Option<i64>,
    wait: Option<u32>,
) -> ApiResult<(Vec<Notice>, u32)> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let account = sqlx::query_scalar!(r#"SELECT account_id FROM agents WHERE id = $1"#, agent_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !user_is_member(&state.db, user.user_id, account).await? {
        return Err(ApiError::Forbidden);
    }
    crate::auth::require_scope(state, user, agent_id).await?;
    state.heartbeats.record(agent_id);

    crate::inbox_wait::claim_waiting(
        state,
        agent_id,
        chakramcp_shared::wake::Feed::Notices,
        wait,
        || claim_rows(state, agent_id, limit),
    )
    .await
}

/// Mark up to `limit` of `agent_id`'s undelivered notices delivered and
/// return them.
async fn claim_rows(state: &RelayState, agent_id: Uuid, limit: i64) -> ApiResult<Vec<Notice>> {
    let rows = sqlx::query!(
        r#"
        WITH claimed AS (
            UPDATE agent_notices n
               SET delivered_at = now()
             WHERE n.id IN (
                   SELECT id FROM agent_notices
                    WHERE agent_id = $1 AND delivered_at IS NULL
                    ORDER BY created_at
                    LIMIT $2
                    FOR UPDATE SKIP LOCKED)
            RETURNING n.id, n.kind, n.payload, n.created_at
        )
        SELECT c.id, c.kind, c.created_at,
               cr.id AS "cr_id?", cr.invocation_id AS "cr_invocation_id?",
               cr.status AS "cr_status?", cr.terms AS "cr_terms?",
               cr.expires_at AS "cr_expires_at?", cr.capability_id AS "cr_capability_id?",
               cr.grantee_agent_id AS "requester_agent_id?",
               cr.grantee_account_id AS "requester_account_id?",
               cap.name AS "capability_name?",
               ea.display_name AS "requester_display_name?",
               eacc.display_name AS "requester_account_display_name?",
               (eacc.verified_at IS NOT NULL) AS "requester_verified?"
          FROM claimed c
          LEFT JOIN consent_requests cr
                 ON cr.id = (c.payload->>'consent_request_id')::uuid
          LEFT JOIN agent_capabilities cap ON cap.id = cr.capability_id
          LEFT JOIN agents ea ON ea.id = cr.grantee_agent_id
          LEFT JOIN accounts eacc ON eacc.id = cr.grantee_account_id
         ORDER BY c.created_at
        "#,
        agent_id,
        limit,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let consent_request = match (
                r.cr_id,
                r.cr_invocation_id,
                r.cr_status,
                r.cr_terms,
                r.cr_expires_at,
                r.cr_capability_id,
                r.requester_agent_id,
                r.requester_account_id,
            ) {
                (
                    Some(id),
                    Some(invocation_id),
                    Some(status),
                    Some(terms),
                    Some(expires_at),
                    Some(capability_id),
                    Some(agent_id),
                    Some(account_id),
                ) => Some(ConsentRequestNotice {
                    id,
                    invocation_id,
                    status,
                    capability_id,
                    capability_name: r.capability_name,
                    requester: NoticeRequester {
                        agent_id,
                        agent_display_name: r.requester_display_name,
                        account_id,
                        account_display_name: r.requester_account_display_name,
                        verified: r.requester_verified.unwrap_or(false),
                    },
                    terms,
                    expires_at,
                    approve_url: approve_url(state, id),
                }),
                _ => None,
            };
            Notice {
                id: r.id,
                kind: r.kind,
                created_at: r.created_at,
                consent_request,
            }
        })
        .collect())
}
