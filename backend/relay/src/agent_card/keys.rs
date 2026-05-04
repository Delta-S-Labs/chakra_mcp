//! Signing-key persistence + JWKS publication.
//!
//! The relay holds Ed25519 keys for signing Agent Cards. Keys live in
//! the `relay_signing_keys` table (created by migration 0011) with a
//! lifecycle of:
//!
//! - **active** (`retired_at IS NULL`): used to sign new cards.
//! - **retired** (`retired_at` set, `expires_at IS NULL` or in the
//!   future): no longer signing, but still in JWKS so previously-
//!   signed cards verify.
//! - **expired** (`expires_at` in the past): no longer in JWKS.
//!   Soft-deleted by leaving the row in place for forensics.
//!
//! v1 simplification: encryption-at-rest is a documented TODO in the
//! discovery spec. This module assumes the DB is the trust boundary
//! for private keys.

use chrono::{DateTime, Utc};
use ed25519_dalek::{SigningKey as DalekSigningKey, SECRET_KEY_LENGTH};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::signer::{
    SigningKey as AppSigningKey, VerifyingKey, ALG_EDDSA, CRV_ED25519, KTY_OKP,
};

/// JWKS document published at `/.well-known/jwks.json`.
///
/// Per RFC 7517. Includes every key that's not yet expired (active
/// + retired-but-still-in-overlap-window). Each entry carries a kid
/// + crv + alg + base64url-encoded public point — verifiers select
/// the entry matching the kid in a JWS protected header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Jwks {
    pub keys: Vec<JsonWebKey>,
}

/// One JOSE Ed25519 public key entry. Matches the OKP family (RFC 8037).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonWebKey {
    /// Key type — "OKP" for Ed25519.
    pub kty: String,
    /// Curve — "Ed25519".
    pub crv: String,
    /// Base64url-encoded 32-byte public key (no padding).
    pub x: String,
    /// Key ID — matches the `kid` in JWS protected headers.
    pub kid: String,
    /// Intended use — always "sig" for our keys.
    #[serde(rename = "use")]
    pub key_use: String,
    /// Algorithm hint — "EdDSA".
    pub alg: String,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("key material was the wrong length (expected {SECRET_KEY_LENGTH} bytes)")]
    WrongLength,
    #[error("no active signing key in DB; call ensure_active_key first")]
    NoActiveKey,
}

pub struct KeyStore {
    pool: PgPool,
}

impl KeyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a fresh randomly-generated Ed25519 key as active. Idempotent
    /// only in the sense that the table will accept multiple active keys —
    /// callers should generally use `ensure_active_key` instead.
    pub async fn create_new_key(&self) -> Result<AppSigningKey, KeyStoreError> {
        let kid = Uuid::now_v7();
        let signing_key = DalekSigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let secret_bytes = signing_key.to_bytes();
        let public_bytes = verifying_key.to_bytes();

        sqlx::query!(
            r#"
            INSERT INTO relay_signing_keys (kid, private_key_bytes, public_key_bytes)
            VALUES ($1, $2, $3)
            "#,
            kid,
            &secret_bytes[..],
            &public_bytes[..],
        )
        .execute(&self.pool)
        .await?;

        Ok(AppSigningKey::from_bytes(kid.to_string(), &secret_bytes))
    }

    /// Ensure the table has at least one active (non-retired) key.
    /// Returns the most recent active key. Generates one if needed.
    /// Safe to call concurrently from multiple replicas — racing
    /// inserts both succeed and the most-recent-by-created_at wins on
    /// the next read.
    pub async fn ensure_active_key(&self) -> Result<AppSigningKey, KeyStoreError> {
        if let Some(k) = self.current_active_key().await? {
            return Ok(k);
        }
        self.create_new_key().await
    }

    /// The most recent non-retired key. None if no active key exists
    /// (should be rare — `ensure_active_key` covers that case).
    pub async fn current_active_key(&self) -> Result<Option<AppSigningKey>, KeyStoreError> {
        let row = sqlx::query!(
            r#"
            SELECT kid, private_key_bytes
              FROM relay_signing_keys
             WHERE retired_at IS NULL
             ORDER BY created_at DESC
             LIMIT 1
            "#
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else { return Ok(None); };
        let bytes_vec = row.private_key_bytes;
        if bytes_vec.len() != SECRET_KEY_LENGTH {
            return Err(KeyStoreError::WrongLength);
        }
        let mut bytes = [0u8; SECRET_KEY_LENGTH];
        bytes.copy_from_slice(&bytes_vec);
        Ok(Some(AppSigningKey::from_bytes(row.kid.to_string(), &bytes)))
    }

    /// All keys that should appear in JWKS — active plus retired-but-
    /// not-yet-expired. Used by the `/.well-known/jwks.json` handler.
    pub async fn jwks_keys(&self) -> Result<Vec<VerifyingKey>, KeyStoreError> {
        let rows = sqlx::query!(
            r#"
            SELECT kid, public_key_bytes
              FROM relay_signing_keys
             WHERE expires_at IS NULL OR expires_at > now()
             ORDER BY created_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            let bytes_vec = row.public_key_bytes;
            if bytes_vec.len() != 32 {
                return Err(KeyStoreError::WrongLength);
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&bytes_vec);
            let inner = ed25519_dalek::VerifyingKey::from_bytes(&bytes)
                .map_err(|_| KeyStoreError::WrongLength)?;
            keys.push(VerifyingKey {
                kid: row.kid.to_string(),
                inner,
            });
        }
        Ok(keys)
    }

    /// Build a JWKS document from `jwks_keys()`.
    pub async fn jwks(&self) -> Result<Jwks, KeyStoreError> {
        let keys = self.jwks_keys().await?;
        Ok(Jwks {
            keys: keys.into_iter().map(verifying_key_to_jwk).collect(),
        })
    }

    /// Mark a key as retired (no longer signing) effective `now()`
    /// (Postgres-side, not Rust-side, to avoid clock-drift ordering
    /// issues against `created_at`). Optionally sets `expires_at`
    /// (when it leaves JWKS). Used by the rotation background job
    /// (D2e) — exposed here for tests / admin tooling.
    ///
    /// `expires_at`, when supplied, MUST be >= the row's existing
    /// `created_at` (CHECK enforced at the DB layer). Callers that
    /// want to mark a key as immediately expired can pass
    /// `Some(Utc::now())` knowing the CHECK will accept it because
    /// Postgres `now()` for the UPDATE is also the new `retired_at`
    /// and equals or precedes the `expires_at` parameter.
    pub async fn retire_key(
        &self,
        kid: Uuid,
        expires_at: Option<DateTime<Utc>>,
        reason: Option<&str>,
    ) -> Result<(), KeyStoreError> {
        sqlx::query!(
            r#"
            UPDATE relay_signing_keys
               SET retired_at = now(),
                   expires_at = $2,
                   rotation_reason = COALESCE($3, rotation_reason)
             WHERE kid = $1
            "#,
            kid,
            expires_at,
            reason,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Serialize the Ed25519 public key as base64url, no padding (RFC 8037 §2).
fn verifying_key_to_jwk(key: VerifyingKey) -> JsonWebKey {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let public_bytes = key.inner.to_bytes();
    JsonWebKey {
        kty: KTY_OKP.to_string(),
        crv: CRV_ED25519.to_string(),
        x: URL_SAFE_NO_PAD.encode(public_bytes),
        kid: key.kid,
        key_use: "sig".to_string(),
        alg: ALG_EDDSA.to_string(),
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests using `#[sqlx::test]`.
    //!
    //! Each test gets its own temp database that the macro creates,
    //! runs `../migrations/*.sql` against, and tears down after.
    //! Requires a live Postgres pointed at by DATABASE_URL (the same
    //! one a developer would have running for `task dev:backend`),
    //! plus a "template" DB the macro can clone — sqlx handles both
    //! automatically.

    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    #[sqlx::test(migrations = "../migrations")]
    async fn ensure_active_key_creates_one_if_missing(pool: PgPool) {
        let store = KeyStore::new(pool);
        let key = store.ensure_active_key().await.unwrap();
        assert!(!key.kid.is_empty());
        let key2 = store.ensure_active_key().await.unwrap();
        assert_eq!(key.kid, key2.kid);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn jwks_includes_active_keys(pool: PgPool) {
        let store = KeyStore::new(pool);
        let key = store.ensure_active_key().await.unwrap();
        let jwks = store.jwks().await.unwrap();
        assert!(jwks.keys.iter().any(|k| k.kid == key.kid));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn jwks_entry_shape_matches_rfc8037(pool: PgPool) {
        let store = KeyStore::new(pool);
        let _ = store.ensure_active_key().await.unwrap();
        let jwks = store.jwks().await.unwrap();
        let key = &jwks.keys[0];
        assert_eq!(key.kty, "OKP");
        assert_eq!(key.crv, "Ed25519");
        assert_eq!(key.alg, "EdDSA");
        assert_eq!(key.key_use, "sig");
        let bytes = URL_SAFE_NO_PAD.decode(&key.x).unwrap();
        assert_eq!(bytes.len(), 32);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn retired_key_stays_in_jwks_until_expired(pool: PgPool) {
        let store = KeyStore::new(pool.clone());

        // Active key — should be in JWKS.
        let active_key = store.ensure_active_key().await.unwrap();
        let active_kid: Uuid = active_key.kid.parse().unwrap();

        // Retire it but leave expires_at NULL — still in JWKS during overlap.
        store.retire_key(active_kid, None, Some("test")).await.unwrap();
        let jwks = store.jwks().await.unwrap();
        assert!(jwks.keys.iter().any(|k| k.kid == active_key.kid));

        // Insert an *already-expired* key directly. The DB CHECK
        // requires created_at <= retired_at <= expires_at, so we
        // backdate all three rather than calling retire_key (which
        // operates on the live row's created_at).
        let expired_kid = Uuid::now_v7();
        let expired_priv = vec![0u8; 32];
        let expired_pub = vec![0u8; 32];
        let one_yr_ago = Utc::now() - chrono::Duration::days(365);
        let six_mo_ago = Utc::now() - chrono::Duration::days(180);
        let one_mo_ago = Utc::now() - chrono::Duration::days(30);
        sqlx::query!(
            r#"
            INSERT INTO relay_signing_keys
                (kid, private_key_bytes, public_key_bytes,
                 created_at, retired_at, expires_at, rotation_reason)
            VALUES ($1, $2, $3, $4, $5, $6, 'test-expired')
            "#,
            expired_kid,
            &expired_priv,
            &expired_pub,
            one_yr_ago,
            six_mo_ago,
            one_mo_ago,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Expired key absent from JWKS; active retired-but-unexpired present.
        let jwks = store.jwks().await.unwrap();
        assert!(jwks.keys.iter().any(|k| k.kid == active_key.kid));
        assert!(!jwks.keys.iter().any(|k| k.kid == expired_kid.to_string()));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn current_active_skips_retired(pool: PgPool) {
        let store = KeyStore::new(pool);
        let k1 = store.ensure_active_key().await.unwrap();
        let kid1: Uuid = k1.kid.parse().unwrap();

        store.retire_key(kid1, None, None).await.unwrap();
        let k2 = store.ensure_active_key().await.unwrap();
        assert_ne!(k1.kid, k2.kid);

        let active = store.current_active_key().await.unwrap().unwrap();
        assert_eq!(active.kid, k2.kid);
    }
}
