//! What an inbox claim adds for consent-gated invocations (spec §4.5, §6).
//!
//! A request token is minted when the inbox first delivers the invocation,
//! whether an approver approved it or a standing consent covered it, so an
//! agent that polls rarely never receives an already-expired token. The
//! token is stored on the consent request; later reads return it.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::token::{self, ConsentClaims};
use crate::state::RelayState;

/// `consent` on an invocation: what was approved, and the token proving it.
#[derive(Debug, Clone, Serialize)]
pub struct ConsentDto {
    pub consent_request_id: Uuid,
    /// The standing consent that covered the request, if one did.
    pub consent_id: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
    pub terms: Value,
    /// Null until the invocation is delivered.
    pub token: Option<String>,
}

/// Mint and store tokens for the approved gate-1 requests among the claimed
/// `ids`, and return each one's `consent`. Invocations without a consent
/// request aren't in the map.
pub async fn deliver(
    state: &RelayState,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, ConsentDto>, ApiError> {
    let rows = sqlx::query!(
        r#"
        SELECT cr.id, cr.invocation_id, cr.capability_id, cr.granter_agent_id,
               cr.grantee_agent_id, cr.consent_id, cr.decided_at, cr.decided_by_user_id,
               COALESCE(cr.approved_terms, cr.terms) AS "terms!",
               cr.token, i.input_preview,
               c.approved_by_user_id AS "consent_approver?"
          FROM consent_requests cr
          JOIN relay_invocations i ON i.id = cr.invocation_id
          LEFT JOIN consents c ON c.id = cr.consent_id
         WHERE cr.invocation_id = ANY($1)
           AND cr.kind = 'request'
           AND cr.status = 'approved'
        "#,
        ids,
    )
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let now = Utc::now();
    let issuer = state.config.relay_base_url.trim_end_matches('/').to_owned();
    let mut minted_ids = Vec::new();
    let mut minted_tokens = Vec::new();
    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let mut token = r.token;
        let approver = r.decided_by_user_id.or(r.consent_approver);
        if let (None, Some(approver)) = (&token, approver) {
            let claims = ConsentClaims {
                iss: issuer.clone(),
                jti: r.id.to_string(),
                kind: "request".into(),
                inv: r.invocation_id.to_string(),
                cap: r.capability_id.to_string(),
                granter: r.granter_agent_id.to_string(),
                grantee: r.grantee_agent_id.to_string(),
                approver: approver.to_string(),
                consent: r.consent_id.map(|c| c.to_string()),
                sha256: token::sha256_jcs(r.input_preview.as_ref().unwrap_or(&Value::Null)),
                iat: now.timestamp(),
                exp: now.timestamp() + token::REQUEST_TOKEN_TTL_SECS,
            };
            let key = state
                .consent_signer
                .key()
                .await
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("consent signing key: {e}")))?;
            let minted = token::mint(&claims, &key)
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("consent token: {e}")))?;
            minted_ids.push(r.id);
            minted_tokens.push(minted.clone());
            token = Some(minted);
        }
        out.insert(
            r.invocation_id,
            ConsentDto {
                consent_request_id: r.id,
                consent_id: r.consent_id,
                approved_at: r.decided_at,
                terms: r.terms,
                token,
            },
        );
    }

    if !minted_ids.is_empty() {
        sqlx::query!(
            r#"
            UPDATE consent_requests c
               SET token = t.token
              FROM unnest($1::uuid[], $2::text[]) AS t(id, token)
             WHERE c.id = t.id AND c.token IS NULL
            "#,
            &minted_ids,
            &minted_tokens,
        )
        .execute(&state.db)
        .await?;
    }
    Ok(out)
}

/// `consent` for an invocation read (list, get): the approved request as
/// stored. `None` unless the gate-1 request was approved.
pub fn read_dto(
    request_id: Option<Uuid>,
    status: Option<&str>,
    consent_id: Option<Uuid>,
    decided_at: Option<DateTime<Utc>>,
    terms: Option<Value>,
    token: Option<String>,
) -> Option<ConsentDto> {
    match (request_id, status) {
        (Some(id), Some("approved")) => Some(ConsentDto {
            consent_request_id: id,
            consent_id,
            approved_at: decided_at,
            terms: terms.unwrap_or(Value::Null),
            token,
        }),
        _ => None,
    }
}
