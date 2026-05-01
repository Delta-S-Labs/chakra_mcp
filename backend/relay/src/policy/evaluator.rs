//! Policy evaluator — runs the decision tree end-to-end.
//!
//! Decoupled from axum extractors: takes a HeaderMap + state + path
//! parts, returns a Decision. The handler in `handlers/a2a.rs` wraps
//! this with the JSON-RPC envelope.

use axum::http::{header::AUTHORIZATION, HeaderMap};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::jwt;

use super::decision::{Authorized, Decision, DenyReason};
use crate::state::RelayState;

/// HTTP header that names which agent the caller is acting as. ChakraMCP
/// API keys are per-account, so callers must specify which of the
/// account's agents is the caller. Future: per-agent JWTs let us
/// derive this from the bearer's claims.
pub const CALLER_AGENT_HEADER: &str = "X-ChakraMCP-Caller-Agent";

/// HTTP header that names the capability being invoked. A2A's wire
/// protocol doesn't have a "skill_id" field on SendMessage, so we
/// require this as a side-channel for policy. ChakraMCP SDKs add it
/// transparently; generic A2A clients calling our relay must include it.
pub const CAPABILITY_HEADER: &str = "X-ChakraMCP-Capability";

/// Push-mode "fresh enough" threshold for TargetUnreachable. The
/// refresh job runs every 60s with a 60min staleness window, so an
/// agent whose card hasn't been fetched for 4h is genuinely missing.
/// Pull-mode targets always pass this gate; D5's inbox bridge takes
/// over once a call is parked.
const PUSH_UNREACHABLE_AFTER_SECONDS: i64 = 4 * 60 * 60;

/// Run the policy gate. Order of checks matches `policy::mod.rs`'s
/// list. Each query is independent so we don't read DB state we
/// won't need on early failure.
pub async fn evaluate(
    db: &PgPool,
    headers: &HeaderMap,
    state: &RelayState,
    target_account_slug: &str,
    target_agent_slug: &str,
) -> Decision {
    // ── 1-2: bearer ───────────────────────────────────────
    let Some(bearer) = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return Decision::Denied(DenyReason::AuthMissing);
    };
    let caller = match resolve_bearer(bearer, state).await {
        Ok(Some(c)) => c,
        Ok(None) => return Decision::Denied(DenyReason::AuthInvalid),
        Err(e) => {
            tracing::error!(error = %e, "bearer resolution failed");
            return Decision::Denied(DenyReason::AuthInvalid);
        }
    };

    // ── 3-4: caller agent ─────────────────────────────────
    let Some(caller_agent_id_str) = headers.get(CALLER_AGENT_HEADER).and_then(|v| v.to_str().ok())
    else {
        return Decision::Denied(DenyReason::CallerAgentHeaderMissing);
    };
    let Ok(caller_agent_id) = caller_agent_id_str.parse::<Uuid>() else {
        return Decision::Denied(DenyReason::CallerAgentNotOwnedByCaller);
    };
    let caller_agent = match sqlx::query!(
        r#"
        SELECT a.id, a.account_id
          FROM agents a
          JOIN account_memberships m ON m.account_id = a.account_id
         WHERE a.id = $1
           AND m.user_id = $2
           AND a.tombstoned_at IS NULL
        "#,
        caller_agent_id,
        caller.user_id,
    )
    .fetch_optional(db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return Decision::Denied(DenyReason::CallerAgentNotOwnedByCaller),
        Err(e) => {
            tracing::error!(error = %e, "caller-agent lookup failed");
            return Decision::Denied(DenyReason::CallerAgentNotOwnedByCaller);
        }
    };

    // ── 5-6: capability header ────────────────────────────
    let Some(cap_id_str) = headers.get(CAPABILITY_HEADER).and_then(|v| v.to_str().ok()) else {
        return Decision::Denied(DenyReason::CapabilityHeaderMissing);
    };
    let Ok(capability_id) = cap_id_str.parse::<Uuid>() else {
        return Decision::Denied(DenyReason::CapabilityHeaderInvalid);
    };

    // ── 7: target account ─────────────────────────────────
    let target_account = match sqlx::query!(
        r#"SELECT id FROM accounts WHERE slug = $1 AND tombstoned_at IS NULL"#,
        target_account_slug,
    )
    .fetch_optional(db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return Decision::Denied(DenyReason::TargetAccountNotFound),
        Err(e) => {
            tracing::error!(error = %e, "target-account lookup failed");
            return Decision::Denied(DenyReason::TargetAccountNotFound);
        }
    };

    // ── 8-9: target agent (incl. tombstone surface) ───────
    // We pull the row (including tombstoned_at) so we can return
    // the more specific TargetTombstoned vs TargetAgentNotFound.
    let target = match sqlx::query!(
        r#"
        SELECT id, mode, tombstoned_at, last_polled_at,
               agent_card_fetched_at
          FROM agents
         WHERE account_id = $1
           AND slug = $2
        "#,
        target_account.id,
        target_agent_slug,
    )
    .fetch_optional(db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return Decision::Denied(DenyReason::TargetAgentNotFound),
        Err(e) => {
            tracing::error!(error = %e, "target-agent lookup failed");
            return Decision::Denied(DenyReason::TargetAgentNotFound);
        }
    };
    if target.tombstoned_at.is_some() {
        return Decision::Denied(DenyReason::TargetTombstoned);
    }

    // ── 10: capability belongs to target ──────────────────
    let cap_owned = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM agent_capabilities WHERE id = $1 AND agent_id = $2)"#,
        capability_id,
        target.id,
    )
    .fetch_one(db)
    .await
    .unwrap_or(Some(false))
    .unwrap_or(false);
    if !cap_owned {
        return Decision::Denied(DenyReason::CapabilityNotOnTarget);
    }

    // ── 11: friendship between caller and target agents ──
    // Friendships are bidirectional in semantics ("we're friends")
    // but stored directionally (proposer/target). Either direction
    // counts as long as status='accepted'.
    let friendship_ok = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM friendships
             WHERE status = 'accepted'
               AND ((proposer_agent_id = $1 AND target_agent_id = $2)
                 OR (proposer_agent_id = $2 AND target_agent_id = $1))
        )
        "#,
        caller_agent.id,
        target.id,
    )
    .fetch_one(db)
    .await
    .unwrap_or(Some(false))
    .unwrap_or(false);
    if !friendship_ok {
        return Decision::Denied(DenyReason::FriendshipRequired);
    }

    // ── 12: active grant on (granter=target, grantee=caller, capability) ─
    let grant_id = match sqlx::query_scalar!(
        r#"
        SELECT id
          FROM grants
         WHERE granter_agent_id = $1
           AND grantee_agent_id = $2
           AND capability_id    = $3
           AND status = 'active'
           AND (expires_at IS NULL OR expires_at > now())
         ORDER BY granted_at DESC
         LIMIT 1
        "#,
        target.id,
        caller_agent.id,
        capability_id,
    )
    .fetch_optional(db)
    .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return Decision::Denied(DenyReason::GrantRequired),
        Err(e) => {
            tracing::error!(error = %e, "grant lookup failed");
            return Decision::Denied(DenyReason::GrantRequired);
        }
    };

    // ── 13: target health ─────────────────────────────────
    // Push: card must have been fetched within the threshold.
    // Pull: always passes; D5's inbox bridge handles parking-on-stale
    // semantics (a parked call to a dormant pull agent times out via
    // the bridge, not via this gate).
    let target_is_push = target.mode == "push";
    if target_is_push {
        let threshold_passed = match target.agent_card_fetched_at {
            Some(t) => {
                let age = chrono::Utc::now() - t;
                age.num_seconds() > PUSH_UNREACHABLE_AFTER_SECONDS
            }
            None => true, // never fetched = unreachable (D5 lazy fetch races
                          // shouldn't gate on this; D2c lazy-fetches before this
                          // path is reached for first-time push agents).
        };
        if threshold_passed {
            return Decision::Denied(DenyReason::TargetUnreachable);
        }
    }

    // ── Pass ──────────────────────────────────────────────
    Decision::Authorized(Authorized {
        caller_user_id: caller.user_id,
        caller_account_id: caller_agent.account_id,
        caller_agent_id: caller_agent.id,
        target_account_id: target_account.id,
        target_agent_id: target.id,
        capability_id,
        grant_id,
        target_is_push,
    })
}

/// Internal: who the bearer resolves to.
struct CallerIdentity {
    user_id: Uuid,
}

/// Resolve `(caller_agent_id)` for a GetTask call. Reuses the same
/// bearer + X-ChakraMCP-Caller-Agent path as the full policy gate
/// but skips capability/friendship/grant — GetTask is a per-task
/// read where authorization is "same caller as the original
/// SendMessage", enforced inside `inbox_bridge::get_task` against
/// the row's grantee_agent_id.
pub async fn resolve_caller_agent_for_get_task(
    db: &sqlx::PgPool,
    headers: &axum::http::HeaderMap,
    state: &RelayState,
) -> Result<Uuid, super::DenyReason> {
    use super::DenyReason;

    let Some(bearer) = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return Err(DenyReason::AuthMissing);
    };
    let caller = match resolve_bearer(bearer, state).await {
        Ok(Some(c)) => c,
        Ok(None) => return Err(DenyReason::AuthInvalid),
        Err(_) => return Err(DenyReason::AuthInvalid),
    };
    let Some(caller_agent_id_str) = headers.get(CALLER_AGENT_HEADER).and_then(|v| v.to_str().ok())
    else {
        return Err(DenyReason::CallerAgentHeaderMissing);
    };
    let Ok(caller_agent_id) = caller_agent_id_str.parse::<Uuid>() else {
        return Err(DenyReason::CallerAgentNotOwnedByCaller);
    };
    let owns = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM agents a
              JOIN account_memberships m ON m.account_id = a.account_id
             WHERE a.id = $1
               AND m.user_id = $2
               AND a.tombstoned_at IS NULL
        )
        "#,
        caller_agent_id,
        caller.user_id,
    )
    .fetch_one(db)
    .await
    .unwrap_or(Some(false))
    .unwrap_or(false);
    if !owns {
        return Err(DenyReason::CallerAgentNotOwnedByCaller);
    }
    Ok(caller_agent_id)
}

async fn resolve_bearer(
    token: &str,
    state: &RelayState,
) -> Result<Option<CallerIdentity>, sqlx::Error> {
    // Try JWT first (cheap, no DB).
    if let Ok(claims) = jwt::decode_jwt(token, state.jwt_secret()) {
        return Ok(Some(CallerIdentity {
            user_id: claims.sub,
        }));
    }
    // Then API key.
    if let Some(stripped) = token.strip_prefix("ck_").map(|_| token) {
        let mut hasher = Sha256::new();
        hasher.update(stripped.as_bytes());
        let key_hash = hex::encode(hasher.finalize());
        let row = sqlx::query!(
            r#"
            SELECT k.user_id
              FROM api_keys k
             WHERE k.key_hash = $1
               AND k.revoked_at IS NULL
               AND (k.expires_at IS NULL OR k.expires_at > now())
             LIMIT 1
            "#,
            key_hash
        )
        .fetch_optional(&state.db)
        .await?;
        if let Some(row) = row {
            return Ok(Some(CallerIdentity {
                user_id: row.user_id,
            }));
        }
    }
    Ok(None)
}

