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
