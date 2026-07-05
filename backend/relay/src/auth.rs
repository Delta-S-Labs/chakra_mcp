//! Auth extractor for the relay service.
//!
//! Validates either a user JWT (issued by `chakramcp-app`) or a personal
//! API key. The two services share `JWT_SECRET` so app-minted tokens
//! work here without re-issuing.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;
use chakramcp_shared::jwt;

use crate::state::RelayState;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    #[allow(dead_code)]
    pub email: String,
    #[allow(dead_code)]
    pub is_admin: bool,
    /// `api_keys.id` if the caller authenticated with a `ck_` token,
    /// `None` if they authenticated with a user JWT (web session,
    /// OAuth, device flow). Recorded on every `relay_invocations`
    /// row this user creates so `/v1/api-keys/{id}/usage` can attribute
    /// traffic per-key.
    pub api_key_id: Option<Uuid>,
    /// JWT id of the bearer that authorised this request, if the
    /// caller used a user JWT. `None` for API-key callers. Recorded
    /// on every `relay_invocations` row so the per-pair usage
    /// dashboard at `/v1/pairings/{kind}/{id}/usage` can attribute
    /// traffic — joins against `oauth_device_codes.minted_jti` and
    /// `oauth_authorizations.minted_jti` (migration 0017).
    pub minted_jti: Option<Uuid>,
}

impl FromRequestParts<RelayState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &RelayState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        authenticate(state, header)
            .await
            .ok_or(ApiError::Unauthorized)
    }
}

/// Resolve an `Authorization` header value into an [`AuthUser`], or `None`
/// if it's missing/invalid/revoked. Shared by the extractor and the usage
/// middleware (which needs to attribute requests without going through the
/// extractor machinery).
pub async fn authenticate(state: &RelayState, auth_header: Option<&str>) -> Option<AuthUser> {
    let token = auth_header?.strip_prefix("Bearer ")?;

    if let Ok(claims) = jwt::decode_jwt(token, state.jwt_secret()) {
        // Revocation honored at both services (shared JWT_SECRET).
        if is_token_revoked(&state.db, claims.jti)
            .await
            .unwrap_or(true)
        {
            return None;
        }
        return Some(AuthUser {
            user_id: claims.sub,
            email: claims.email,
            is_admin: claims.is_admin,
            api_key_id: None,
            minted_jti: Some(claims.jti),
        });
    }

    api_key_lookup(&state.db, token).await.ok().flatten()
}

async fn is_token_revoked(db: &PgPool, jti: Uuid) -> Result<bool, ApiError> {
    let row = sqlx::query!(
        r#"SELECT 1 as one FROM revoked_tokens WHERE jti = $1 LIMIT 1"#,
        jti,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

async fn api_key_lookup(db: &PgPool, token: &str) -> Result<Option<AuthUser>, ApiError> {
    if !token.starts_with("ck_") {
        return Ok(None);
    }
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    let row = sqlx::query!(
        r#"
        SELECT k.id as key_id, u.id as user_id, u.email, u.is_admin
        FROM api_keys k
        JOIN users u ON u.id = k.user_id
        WHERE k.key_hash = $1
          AND k.revoked_at IS NULL
          AND (k.expires_at IS NULL OR k.expires_at > now())
        LIMIT 1
        "#,
        key_hash
    )
    .fetch_optional(db)
    .await?;

    if let Some(r) = row {
        let _ = sqlx::query!(
            r#"UPDATE api_keys SET last_used_at = now() WHERE key_hash = $1"#,
            key_hash
        )
        .execute(db)
        .await;

        Ok(Some(AuthUser {
            user_id: r.user_id,
            email: r.email,
            is_admin: r.is_admin,
            api_key_id: Some(r.key_id),
            // API-key path — no JWT involved, so no jti to record.
            minted_jti: None,
        }))
    } else {
        Ok(None)
    }
}

/// Returns true if the user is a member of the given account.
pub async fn user_is_member(
    db: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<bool, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT 1 as one FROM account_memberships
        WHERE user_id = $1 AND account_id = $2
        LIMIT 1
        "#,
        user_id,
        account_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

/// Resolve the OAuth client that minted the caller's JWT, if any.
///
/// Looks the token's `jti` up against the two grant tables that record
/// `minted_jti` + `client_id` (the auth-code flow's `oauth_authorizations`
/// and the device flow's `oauth_device_codes`). Returns `None` for API-key
/// callers, plain web-session tokens, or tokens with no recorded lineage.
///
/// This is an extra query, so only call it on management endpoints (agent
/// create / scope enforcement) — never on the invocation hot path.
pub async fn caller_client_id(
    db: &PgPool,
    minted_jti: Option<Uuid>,
) -> Result<Option<String>, ApiError> {
    let Some(jti) = minted_jti else {
        return Ok(None);
    };
    let row = sqlx::query_scalar!(
        r#"
        SELECT client_id FROM oauth_authorizations WHERE minted_jti = $1
        UNION
        SELECT client_id FROM oauth_device_codes   WHERE minted_jti = $1
        LIMIT 1
        "#,
        jti,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.flatten())
}

/// Returns true if the user is an owner or admin of the given account.
pub async fn user_can_admin_account(
    db: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<bool, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT role FROM account_memberships
        WHERE user_id = $1 AND account_id = $2
        LIMIT 1
        "#,
        user_id,
        account_id
    )
    .fetch_optional(db)
    .await?;
    Ok(matches!(row.map(|r| r.role), Some(ref r) if r == "owner" || r == "admin"))
}

// ─── Per-credential agent-management scope (migration 0027) ──────────
//
// The scope model + guard now live in `chakramcp_shared::scope` so the app
// service (agent re-parenting) enforces it through the same implementation
// the relay uses for agent / capability / friendship / grant management.
// Re-exported here so existing `crate::auth::…` call sites are unchanged.
pub use chakramcp_shared::scope::{grant_allows_agent, AgentScopeMode, GrantScope};

/// Resolve the caller's agent-management scope. Thin wrapper over
/// `chakramcp_shared::scope::resolve_grant` that pulls the credential
/// identity (JWT jti / API key id) out of the relay's `AuthUser`.
pub async fn resolve_grant(db: &PgPool, user: &AuthUser) -> Result<GrantScope, ApiError> {
    chakramcp_shared::scope::resolve_grant(db, user.minted_jti, user.api_key_id).await
}

// `grant_allows_agent` is re-exported from `chakramcp_shared::scope` above.

#[cfg(test)]
mod tests {
    use super::*;

    // Seeds a user, account, two OAuth clients (mcp_a / mcp_b), an agent
    // attributed to each, and a 'selected' scope enrolling agent_b.
    // Returns (user_id, agent_a, agent_b, selected_scope_id). Fixtures use
    // the runtime query API so they don't add entries to the .sqlx cache.
    async fn seed(pool: &PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
        let user = Uuid::now_v7();
        sqlx::query("INSERT INTO users (id, email, display_name) VALUES ($1, $2, $3)")
            .bind(user)
            .bind(format!("u-{user}@test.test"))
            .bind("U")
            .execute(pool)
            .await
            .unwrap();

        let account = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id) \
             VALUES ($1, $2, $3, 'individual', $4)",
        )
        .bind(account)
        .bind(format!("a-{account}"))
        .bind("A")
        .bind(user)
        .execute(pool)
        .await
        .unwrap();

        for c in ["mcp_a", "mcp_b"] {
            sqlx::query(
                "INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::now_v7())
            .bind(c)
            .bind(c)
            .bind(vec!["https://x.test".to_string()])
            .execute(pool)
            .await
            .unwrap();
        }

        let mk_agent = |id: Uuid, slug: &'static str, client: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO agents (id, account_id, slug, display_name, mode, created_by_client_id) \
                     VALUES ($1, $2, $3, $4, 'pull', $5)",
                )
                .bind(id)
                .bind(account)
                .bind(slug)
                .bind(slug)
                .bind(client)
                .execute(&pool)
                .await
                .unwrap();
            }
        };
        let agent_a = Uuid::now_v7();
        let agent_b = Uuid::now_v7();
        mk_agent(agent_a, "agent-a", "mcp_a").await;
        mk_agent(agent_b, "agent-b", "mcp_b").await;

        let scope_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO credential_scopes (id, cred_kind, cred_ref, user_id, agent_scope) \
             VALUES ($1, 'jwt', $2, $3, 'selected')",
        )
        .bind(scope_id)
        .bind(format!("sel-{scope_id}"))
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO credential_scope_agents (credential_scope_id, agent_id) VALUES ($1, $2)",
        )
        .bind(scope_id)
        .bind(agent_b)
        .execute(pool)
        .await
        .unwrap();

        (user, agent_a, agent_b, scope_id)
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn all_scope_allows_any_agent(pool: PgPool) {
        let (_u, agent_a, agent_b, _s) = seed(&pool).await;
        let grant = GrantScope {
            scope_id: None,
            mode: AgentScopeMode::All,
            client_id: None,
            api_key_id: None,
        };
        assert!(grant_allows_agent(&pool, &grant, agent_a).await.unwrap());
        assert!(grant_allows_agent(&pool, &grant, agent_b).await.unwrap());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn own_scope_allows_only_clients_own_agents(pool: PgPool) {
        let (_u, agent_a, agent_b, _s) = seed(&pool).await;
        let grant = GrantScope {
            scope_id: None,
            mode: AgentScopeMode::Own,
            client_id: Some("mcp_a".to_string()),
            api_key_id: None,
        };
        assert!(
            grant_allows_agent(&pool, &grant, agent_a).await.unwrap(),
            "client A may manage the agent it created"
        );
        assert!(
            !grant_allows_agent(&pool, &grant, agent_b).await.unwrap(),
            "client A may NOT manage an agent client B created"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn selected_scope_allows_only_enrolled_agents(pool: PgPool) {
        let (_u, agent_a, agent_b, scope_id) = seed(&pool).await;
        let grant = GrantScope {
            scope_id: Some(scope_id),
            mode: AgentScopeMode::Selected,
            client_id: None,
            api_key_id: None,
        };
        assert!(
            grant_allows_agent(&pool, &grant, agent_b).await.unwrap(),
            "enrolled agent is allowed"
        );
        assert!(
            !grant_allows_agent(&pool, &grant, agent_a).await.unwrap(),
            "non-enrolled agent is denied"
        );
    }

    // Adversarial: `own` must FAIL CLOSED on an agent with NULL attribution
    // (e.g. one created via the web dashboard). A NULL never equals a
    // client_id, so such an agent is invisible to an `own` credential —
    // the safe direction (sees fewer agents, never more).
    #[sqlx::test(migrations = "../migrations")]
    async fn own_denies_agent_with_null_attribution(pool: PgPool) {
        let (user, _a, _b, _s) = seed(&pool).await;
        let acct: Uuid = sqlx::query_scalar("SELECT id FROM accounts WHERE owner_user_id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
        let orphan = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO agents (id, account_id, slug, display_name, mode) \
             VALUES ($1, $2, 'orphan', 'Orphan', 'pull')",
        )
        .bind(orphan)
        .bind(acct)
        .execute(&pool)
        .await
        .unwrap();

        let grant = GrantScope {
            scope_id: None,
            mode: AgentScopeMode::Own,
            client_id: Some("mcp_a".to_string()),
            api_key_id: None,
        };
        assert!(
            !grant_allows_agent(&pool, &grant, orphan).await.unwrap(),
            "own must fail closed on an agent with NULL attribution"
        );
    }

    // Relationship-read scoping: manageable_agent_ids returns the bounded
    // manageable set (None for `all`), and manages_either gates a two-agent
    // relationship to those touching a manageable side.
    #[sqlx::test(migrations = "../migrations")]
    async fn manageable_ids_scope_relationship_reads(pool: PgPool) {
        use chakramcp_shared::scope::{manageable_agent_ids, manages_either};

        let (_u, agent_a, agent_b, scope_id) = seed(&pool).await;

        // 'selected' scope enrolls agent_b (per seed).
        let selected = GrantScope {
            scope_id: Some(scope_id),
            mode: AgentScopeMode::Selected,
            client_id: None,
            api_key_id: None,
        };
        let ids = manageable_agent_ids(&pool, &selected).await.unwrap();
        assert_eq!(ids, Some(vec![agent_b]));
        assert!(
            manages_either(&ids, agent_a, agent_b),
            "relationship touching the enrolled agent is visible"
        );
        assert!(
            !manages_either(&ids, agent_a, Uuid::now_v7()),
            "relationship touching no enrolled agent is hidden"
        );

        // 'own' for client mcp_a resolves to the agent it created (agent_a).
        let own = GrantScope {
            scope_id: None,
            mode: AgentScopeMode::Own,
            client_id: Some("mcp_a".to_string()),
            api_key_id: None,
        };
        assert_eq!(
            manageable_agent_ids(&pool, &own).await.unwrap(),
            Some(vec![agent_a])
        );

        // 'all' → no restriction; every relationship passes.
        let all = manageable_agent_ids(&pool, &GrantScope::unrestricted(None))
            .await
            .unwrap();
        assert_eq!(all, None);
        assert!(manages_either(&all, Uuid::now_v7(), Uuid::now_v7()));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn resolve_grant_defaults_to_all_without_row(pool: PgPool) {
        let user = AuthUser {
            user_id: Uuid::now_v7(),
            email: "x@test.test".to_string(),
            is_admin: false,
            api_key_id: None,
            minted_jti: Some(Uuid::now_v7()),
        };
        let grant = resolve_grant(&pool, &user).await.unwrap();
        assert_eq!(grant.mode, AgentScopeMode::All);
        assert!(grant.scope_id.is_none());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn resolve_grant_reads_bound_scope(pool: PgPool) {
        let (user, _a, _b, _s) = seed(&pool).await;
        let jti = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO credential_scopes (id, cred_kind, cred_ref, client_id, user_id, agent_scope) \
             VALUES ($1, 'jwt', $2, 'mcp_a', $3, 'own')",
        )
        .bind(Uuid::now_v7())
        .bind(jti.to_string())
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();

        let auth = AuthUser {
            user_id: user,
            email: "x@test.test".to_string(),
            is_admin: false,
            api_key_id: None,
            minted_jti: Some(jti),
        };
        let grant = resolve_grant(&pool, &auth).await.unwrap();
        assert_eq!(grant.mode, AgentScopeMode::Own);
        assert_eq!(grant.client_id.as_deref(), Some("mcp_a"));
    }
}
