//! Per-credential agent-management scope (migration 0027).
//!
//! Layered ON TOP of account membership: a scope only ever NARROWS what a
//! credential may manage, never widens it. Absence of a `credential_scopes`
//! row means `All` (the pre-feature default), so credentials minted before
//! this feature — and plain web sessions — are unaffected.
//!
//! This lives in the shared crate, not the relay, so the relay (agent /
//! capability / friendship / grant management) and the app service (agent
//! re-parenting) both resolve scope through ONE implementation. Divergent
//! copies are exactly how a management path can silently skip a gate that a
//! sibling path enforces.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentScopeMode {
    /// Manage any agent in accounts the caller is a member of.
    All,
    /// Manage only agents this credential's issuer created.
    Own,
    /// Manage only the agents enumerated in `credential_scope_agents`.
    Selected,
}

#[derive(Debug, Clone)]
pub struct GrantScope {
    /// `credential_scopes.id` when a scope is bound — needed for the
    /// 'selected' allow-list lookup and for adding freshly-created agents
    /// to it. `None` ⇒ no row ⇒ implicit `All`.
    pub scope_id: Option<Uuid>,
    pub mode: AgentScopeMode,
    /// OAuth client behind the credential (for 'own' over a JWT). `None`
    /// for API-key callers.
    pub client_id: Option<String>,
    /// API key id behind the credential (for 'own' over an API key).
    pub api_key_id: Option<Uuid>,
}

impl GrantScope {
    pub fn unrestricted(api_key_id: Option<Uuid>) -> Self {
        Self {
            scope_id: None,
            mode: AgentScopeMode::All,
            client_id: None,
            api_key_id,
        }
    }

    /// The scope mode as its wire string (`all` | `own` | `selected`).
    /// Lets a LIST query filter by scope in one static SQL fragment —
    /// `$mode = 'all' OR ($mode = 'own' AND …) OR ($mode = 'selected' AND …)`
    /// — instead of a per-row `grant_allows_agent` round-trip.
    pub fn mode_str(&self) -> &'static str {
        match self.mode {
            AgentScopeMode::All => "all",
            AgentScopeMode::Own => "own",
            AgentScopeMode::Selected => "selected",
        }
    }
}

/// Resolve the agent-management scope bound to the caller's credential.
/// Returns `All` when no `credential_scopes` row exists — so any credential
/// minted before this feature (or any plain web session) is unrestricted.
///
/// Takes the caller's JWT `jti` (for jwt credentials) and/or API key id (for
/// api_key credentials) rather than an `AuthUser`, so both services can call
/// it with whatever their own auth extractor produces.
///
/// Call only on management endpoints; it's an extra query, not worth paying
/// on the invocation hot path.
pub async fn resolve_grant(
    db: &PgPool,
    minted_jti: Option<Uuid>,
    api_key_id: Option<Uuid>,
) -> Result<GrantScope, ApiError> {
    let (cred_kind, cred_ref) = if let Some(jti) = minted_jti {
        ("jwt", jti.to_string())
    } else if let Some(key) = api_key_id {
        ("api_key", key.to_string())
    } else {
        return Ok(GrantScope::unrestricted(api_key_id));
    };

    let row = sqlx::query!(
        r#"
        SELECT id, agent_scope, client_id
        FROM credential_scopes
        WHERE cred_kind = $1 AND cred_ref = $2
        LIMIT 1
        "#,
        cred_kind,
        cred_ref,
    )
    .fetch_optional(db)
    .await?;

    let Some(r) = row else {
        return Ok(GrantScope::unrestricted(api_key_id));
    };

    let mode = match r.agent_scope.as_str() {
        "own" => AgentScopeMode::Own,
        "selected" => AgentScopeMode::Selected,
        _ => AgentScopeMode::All,
    };
    Ok(GrantScope {
        scope_id: Some(r.id),
        mode,
        client_id: r.client_id,
        api_key_id,
    })
}

/// Whether `grant` permits MANAGING `agent_id`. Call only after the caller
/// is confirmed a member of the agent's account — this is the extra,
/// scope-level gate that sits on top of membership.
///
/// Fail-closed on NULLs: `own` requires both the grant's identity and the
/// agent's attribution column to be non-NULL and equal, so a NULL on either
/// side matches nothing (never everything).
pub async fn grant_allows_agent(
    db: &PgPool,
    grant: &GrantScope,
    agent_id: Uuid,
) -> Result<bool, ApiError> {
    match grant.mode {
        AgentScopeMode::All => Ok(true),
        AgentScopeMode::Own => {
            let row = sqlx::query!(
                r#"SELECT created_by_client_id, created_by_api_key_id FROM agents WHERE id = $1"#,
                agent_id,
            )
            .fetch_optional(db)
            .await?;
            let Some(r) = row else {
                return Ok(false);
            };
            let by_client = matches!(
                (&grant.client_id, &r.created_by_client_id),
                (Some(g), Some(a)) if g == a
            );
            let by_key = matches!(
                (grant.api_key_id, r.created_by_api_key_id),
                (Some(g), Some(a)) if g == a
            );
            Ok(by_client || by_key)
        }
        AgentScopeMode::Selected => {
            let Some(scope_id) = grant.scope_id else {
                return Ok(false);
            };
            let row = sqlx::query!(
                r#"
                SELECT 1 as one FROM credential_scope_agents
                WHERE credential_scope_id = $1 AND agent_id = $2
                LIMIT 1
                "#,
                scope_id,
                agent_id,
            )
            .fetch_optional(db)
            .await?;
            Ok(row.is_some())
        }
    }
}

/// The bounded set of agent ids a credential may MANAGE, or `None` for
/// `all` (no restriction). Used to scope-filter *relationship* reads
/// (friendships / grants / invocations), where each row involves two
/// agents and should be visible only if the credential manages at least
/// one of them. Membership is enforced separately by the read queries
/// themselves; this only adds the scope narrowing.
pub async fn manageable_agent_ids(
    db: &PgPool,
    grant: &GrantScope,
) -> Result<Option<Vec<Uuid>>, ApiError> {
    let ids = match grant.mode {
        AgentScopeMode::All => return Ok(None),
        AgentScopeMode::Own => {
            sqlx::query_scalar!(
                r#"SELECT id FROM agents
                   WHERE created_by_client_id = $1 OR created_by_api_key_id = $2"#,
                grant.client_id,
                grant.api_key_id,
            )
            .fetch_all(db)
            .await?
        }
        AgentScopeMode::Selected => {
            let Some(scope_id) = grant.scope_id else {
                return Ok(Some(Vec::new()));
            };
            sqlx::query_scalar!(
                r#"SELECT agent_id FROM credential_scope_agents WHERE credential_scope_id = $1"#,
                scope_id,
            )
            .fetch_all(db)
            .await?
        }
    };
    Ok(Some(ids))
}

/// Whether a relationship touching agents `a` and `b` is in scope. `None`
/// (the `all` case) always is; otherwise at least one side must be in the
/// manageable set. Call after the read query/fetch has already confirmed
/// the caller is a member of a side — this only adds the scope gate.
pub fn manages_either(manageable: &Option<Vec<Uuid>>, a: Uuid, b: Uuid) -> bool {
    match manageable {
        None => true,
        Some(ids) => ids.contains(&a) || ids.contains(&b),
    }
}
