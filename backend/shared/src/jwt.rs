use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JWT claims for authenticated humans.
///
/// Issued by the `app` service after sign-in. Both `app` and `relay`
/// validate using the shared JWT_SECRET so the same token works against
/// either service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserClaims {
    /// User id in the `users` table.
    pub sub: Uuid,
    /// Email — convenience for log readability.
    pub email: String,
    /// Whether this user matches ADMIN_EMAIL.
    pub is_admin: bool,
    /// Issued at (unix seconds).
    pub iat: i64,
    /// Expiry (unix seconds).
    pub exp: i64,
    /// Token id — unique UUID v7, used as the primary key of the
    /// `revoked_tokens` table so a leaked JWT can be killed before
    /// natural expiry. Legacy tokens minted before this field was
    /// introduced deserialize with `Uuid::nil()`; the revocation
    /// check is a no-op for those (no row will ever have a nil jti).
    #[serde(default)]
    pub jti: Uuid,
}

impl UserClaims {
    pub fn new(user_id: Uuid, email: String, is_admin: bool, ttl_hours: i64) -> Self {
        let now = Utc::now();
        Self {
            sub: user_id,
            email,
            is_admin,
            iat: now.timestamp(),
            exp: (now + Duration::hours(ttl_hours)).timestamp(),
            jti: Uuid::now_v7(),
        }
    }
}

pub fn encode_jwt(claims: &UserClaims, secret: &str) -> jsonwebtoken::errors::Result<String> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

pub fn decode_jwt(token: &str, secret: &str) -> jsonwebtoken::errors::Result<UserClaims> {
    let data = decode::<UserClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}
