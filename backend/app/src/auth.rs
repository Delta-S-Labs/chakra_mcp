//! Authentication helpers — extract a `UserClaims` from an Axum request.
//!
//! Two ways a request authenticates:
//!   * `Authorization: Bearer <jwt>` — the app's own user JWT.
//!   * `Authorization: Bearer <api_key>` — a personal API key (sha256-hashed
//!     and looked up). Currently unused by the frontend, planned for the
//!     example agents and CLI tooling.
//!
//! The handler decides which it expects via the `AuthUser` extractor.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;
use chakramcp_shared::jwt;

use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub email: String,
    pub is_admin: bool,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;

        let token = header.strip_prefix("Bearer ").ok_or(ApiError::Unauthorized)?;

        // Try JWT first; if that fails, try API key.
        if let Ok(claims) = jwt::decode_jwt(token, &state.config.jwt_secret) {
            return Ok(AuthUser {
                user_id: claims.sub,
                email: claims.email,
                is_admin: claims.is_admin,
            });
        }

        // API-key path: hash and lookup.
        if let Some(user) = api_key_lookup(&state.db, token).await? {
            return Ok(user);
        }

        Err(ApiError::Unauthorized)
    }
}

/// Convenience: same as `AuthUser` but rejects non-admins.
#[derive(Debug, Clone)]
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(ApiError::Forbidden);
        }
        Ok(Self(user))
    }
}

/// Hash an API key with SHA-256 and return the lowercase hex digest.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

async fn api_key_lookup(db: &PgPool, token: &str) -> Result<Option<AuthUser>, ApiError> {
    // Quick rejection: API keys have a known shape we can sniff to avoid
    // hashing every random string. Fall through to JWT path otherwise.
    if !token.starts_with("ck_") {
        return Ok(None);
    }

    let key_hash = hash_api_key(token);
    let row = sqlx::query!(
        r#"
        SELECT u.id as user_id, u.email, u.is_admin
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
        // Best-effort last_used_at update — fire and forget.
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
        }))
    } else {
        Ok(None)
    }
}

/// Generate a random API key plaintext. Format: `ck_<32-byte hex>`.
pub fn generate_api_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("ck_{}", hex::encode(bytes))
}

/// First 9 chars of the key — safe to store in plaintext for display.
pub fn key_prefix(plaintext: &str) -> String {
    plaintext.chars().take(9).collect()
}

/// Helper used by handlers to convert a missing-row into 404 / unauthorized.
pub fn forbid_if_not_self(claims_user_id: Uuid, target_user_id: Uuid) -> Result<(), ApiError> {
    if claims_user_id != target_user_id {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

