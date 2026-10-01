//! Users and their personal accounts, shared by sign-up and the operator
//! commands (`chakramcp-server users`, `chakramcp-server credits`).
//!
//! Passwords are hashed with Argon2id. `users.password_hash` stores the full
//! PHC string (salt and parameters included); plaintext is never stored.

use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

pub const MIN_PASSWORD_LEN: usize = 8;
pub const MAX_PASSWORD_LEN: usize = 200;

/// A user to create. `is_admin` is the caller's decision: sign-up applies
/// the `ADMIN_EMAIL` rule, the operator command its `--admin` flag.
pub struct NewUser<'a> {
    pub email: &'a str,
    pub name: &'a str,
    pub password: &'a str,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct CreatedUser {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
    pub account_id: Uuid,
    pub account_slug: String,
}

/// How lookups compare emails: trimmed and lowercased.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub fn validate_password(password: &str) -> ApiResult<()> {
    if password.len() < MIN_PASSWORD_LEN || password.len() > MAX_PASSWORD_LEN {
        return Err(ApiError::InvalidRequest(format!(
            "password must be {MIN_PASSWORD_LEN}–{MAX_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

/// Create a user with a personal account and its owner membership, in one
/// transaction. The email is stored as typed (trimmed); lookups ignore case.
pub async fn create_user(db: &PgPool, new: NewUser<'_>) -> ApiResult<CreatedUser> {
    let email = normalize_email(new.email);
    if email.is_empty() || !email.contains('@') {
        return Err(ApiError::InvalidRequest("a valid email is required".into()));
    }
    validate_password(new.password)?;
    let name = new.name.trim();
    if name.is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }

    let exists = sqlx::query!(
        r#"SELECT id FROM users WHERE LOWER(email) = $1 LIMIT 1"#,
        email
    )
    .fetch_optional(db)
    .await?;
    if exists.is_some() {
        return Err(ApiError::Conflict(
            "an account with this email already exists".into(),
        ));
    }

    let password_hash = hash_password(new.password)?;
    let mut tx = db.begin().await?;

    let user = sqlx::query!(
        r#"
        INSERT INTO users (id, email, display_name, is_admin, password_hash)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, email, display_name, avatar_url, is_admin
        "#,
        Uuid::now_v7(),
        new.email.trim(),
        name,
        new.is_admin,
        password_hash,
    )
    .fetch_one(&mut *tx)
    .await?;

    let account_id = Uuid::now_v7();
    let account_slug = personal_slug(&user.email);
    sqlx::query!(
        r#"
        INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
        VALUES ($1, $2, $3, 'individual', $4)
        "#,
        account_id,
        account_slug,
        user.display_name,
        user.id,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO account_memberships (id, account_id, user_id, role)
        VALUES ($1, $2, $3, 'owner')
        "#,
        Uuid::now_v7(),
        account_id,
        user.id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(CreatedUser {
        user_id: user.id,
        email: user.email,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
        is_admin: user.is_admin,
        account_id,
        account_slug,
    })
}

/// A user who signed in with their password.
#[derive(Debug, Clone)]
pub struct SignedIn {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
}

/// Check an email and password, under the failed sign-in limit
/// (`crate::signin_limit`). `Unauthorized` for an unknown email, a user with
/// no password, or a wrong one, all alike so the answer doesn't reveal which
/// emails exist; `SigninRateLimited` while the email is over the limit.
pub async fn authenticate(db: &PgPool, email: &str, password: &str) -> ApiResult<SignedIn> {
    let email = normalize_email(email);
    crate::signin_limit::check(db, &email).await?;
    let row = sqlx::query!(
        r#"
        SELECT id, email, display_name, avatar_url, is_admin, password_hash
          FROM users
         WHERE LOWER(email) = $1
         LIMIT 1
        "#,
        email
    )
    .fetch_optional(db)
    .await?;
    let verified = match &row {
        Some(r) => match r.password_hash.as_deref() {
            Some(stored) => verify_password(password, stored),
            None => Err(ApiError::Unauthorized),
        },
        None => Err(ApiError::Unauthorized),
    };
    match verified {
        Ok(()) => {
            crate::signin_limit::clear(db, &email).await?;
            let r = row.expect("verified implies a row");
            Ok(SignedIn {
                user_id: r.id,
                email: r.email,
                display_name: r.display_name,
                avatar_url: r.avatar_url,
                is_admin: r.is_admin,
            })
        }
        Err(ApiError::Unauthorized) => {
            crate::signin_limit::record_failure(db, &email).await?;
            Err(ApiError::Unauthorized)
        }
        Err(other) => Err(other),
    }
}

/// Whether a user has this email (case-insensitive).
pub async fn user_exists(db: &PgPool, email: &str) -> ApiResult<bool> {
    let found = sqlx::query_scalar!(
        r#"SELECT 1 AS "one!" FROM users WHERE LOWER(email) = $1 LIMIT 1"#,
        normalize_email(email),
    )
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// Replace a user's password. Sign-ins made before it stay valid until they
/// expire.
pub async fn set_password(db: &PgPool, email: &str, password: &str) -> ApiResult<()> {
    validate_password(password)?;
    let password_hash = hash_password(password)?;
    sqlx::query!(
        r#"
        UPDATE users SET password_hash = $1, updated_at = now()
         WHERE LOWER(email) = $2
        RETURNING id
        "#,
        password_hash,
        normalize_email(email),
    )
    .fetch_optional(db)
    .await?
    .map(|_| ())
    .ok_or(ApiError::NotFound)
}

/// Set or clear the admin flag. Tokens carry the flag, so a change takes
/// effect at the user's next sign-in.
pub async fn set_admin(db: &PgPool, email: &str, is_admin: bool) -> ApiResult<()> {
    sqlx::query!(
        r#"
        UPDATE users SET is_admin = $1, updated_at = now()
         WHERE LOWER(email) = $2
        RETURNING id
        "#,
        is_admin,
        normalize_email(email),
    )
    .fetch_optional(db)
    .await?
    .map(|_| ())
    .ok_or(ApiError::NotFound)
}

#[derive(Debug, Serialize)]
pub struct UserSummary {
    pub email: String,
    pub name: String,
    pub is_admin: bool,
    /// The personal account's slug: what `credits` commands accept.
    pub account: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Every user, oldest first, with their personal account.
pub async fn list_users(db: &PgPool) -> ApiResult<Vec<UserSummary>> {
    let rows = sqlx::query_as!(
        UserSummary,
        r#"
        SELECT u.email, u.display_name AS name, u.is_admin, u.created_at,
               (SELECT a.slug FROM accounts a
                 WHERE a.owner_user_id = u.id AND a.account_type = 'individual'
                 ORDER BY a.created_at LIMIT 1) AS account
          FROM users u
         ORDER BY u.created_at, u.id
        "#
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// An account named by its slug, its id, or a user's email (that user's
/// personal account).
pub async fn resolve_account(db: &PgPool, input: &str) -> ApiResult<Uuid> {
    let input = input.trim();
    let found = if let Ok(id) = Uuid::parse_str(input) {
        sqlx::query_scalar!("SELECT id FROM accounts WHERE id = $1", id)
            .fetch_optional(db)
            .await?
    } else if input.contains('@') {
        sqlx::query_scalar!(
            r#"
            SELECT a.id FROM accounts a
              JOIN users u ON u.id = a.owner_user_id
             WHERE LOWER(u.email) = $1 AND a.account_type = 'individual'
             ORDER BY a.created_at
             LIMIT 1
            "#,
            normalize_email(input),
        )
        .fetch_optional(db)
        .await?
    } else {
        sqlx::query_scalar!("SELECT id FROM accounts WHERE slug = $1", input)
            .fetch_optional(db)
            .await?
    };
    found.ok_or(ApiError::NotFound)
}

pub fn hash_password(plain: &str) -> ApiResult<String> {
    // argon2 0.6 (password-hash 0.6): `hash_password` takes only the password
    // and generates a random salt itself via the `getrandom` feature (on by
    // default), replacing the old explicit `SaltString::generate(&mut OsRng)`.
    let hash = Argon2::default()
        .hash_password(plain.as_bytes())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("password hashing failed: {e}")))?;
    Ok(hash.to_string())
}

pub fn verify_password(plain: &str, stored: &str) -> ApiResult<()> {
    let parsed = PasswordHash::new(stored)
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("stored password hash is malformed")))?;
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .map_err(|_| ApiError::Unauthorized)
}

/// A slug for a personal account: the part of the email before `@`,
/// lowercased, other characters replaced, plus 8 characters of a new id.
/// A collision would surface as the unique constraint on `accounts.slug`.
pub fn personal_slug(email: &str) -> String {
    let local = email.split('@').next().unwrap_or("user");
    let mut s: String = local
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if s.is_empty() {
        s.push_str("user");
    }
    format!("{}-{}", s, &Uuid::now_v7().simple().to_string()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_user<'a>(email: &'a str, password: &'a str, is_admin: bool) -> NewUser<'a> {
        NewUser {
            email,
            name: "Test User",
            password,
            is_admin,
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn creates_the_user_their_account_and_membership(pool: PgPool) {
        let created = create_user(&pool, new_user(" Ada@Example.test ", "long-enough", true))
            .await
            .unwrap();
        assert_eq!(
            created.email, "Ada@Example.test",
            "stored as typed, trimmed"
        );
        assert!(created.is_admin, "the caller decides the admin flag");
        assert!(created.account_slug.starts_with("ada-"));

        let (owner, role): (Uuid, String) = sqlx::query_as(
            "SELECT a.owner_user_id, m.role FROM accounts a
               JOIN account_memberships m ON m.account_id = a.id
              WHERE a.id = $1 AND a.account_type = 'individual'",
        )
        .bind(created.account_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(owner, created.user_id);
        assert_eq!(role, "owner");

        let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(created.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        verify_password("long-enough", &hash).unwrap();
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn refuses_bad_input_and_duplicates(pool: PgPool) {
        let too_long = "x".repeat(201);
        for (email, password, name) in [
            ("no-at-sign", "long-enough", "A"),
            ("  ", "long-enough", "A"),
            ("a@example.test", "short", "A"),
            ("a@example.test", too_long.as_str(), "A"),
            ("a@example.test", "long-enough", "  "),
        ] {
            let err = create_user(
                &pool,
                NewUser {
                    email,
                    name,
                    password,
                    is_admin: false,
                },
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, ApiError::InvalidRequest(_)),
                "{email:?}/{name:?}: {err:?}"
            );
        }

        create_user(&pool, new_user("a@example.test", "long-enough", false))
            .await
            .unwrap();
        let err = create_user(&pool, new_user("A@EXAMPLE.TEST", "long-enough", false))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)), "{err:?}");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn passwords_and_the_admin_flag_change_by_email(pool: PgPool) {
        let created = create_user(&pool, new_user("b@example.test", "first-password", false))
            .await
            .unwrap();

        set_password(&pool, "B@example.test", "second-password")
            .await
            .unwrap();
        let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(created.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        verify_password("second-password", &hash).unwrap();
        assert!(verify_password("first-password", &hash).is_err());
        assert!(matches!(
            set_password(&pool, "b@example.test", "short").await,
            Err(ApiError::InvalidRequest(_))
        ));

        set_admin(&pool, "b@example.test", true).await.unwrap();
        let listed = list_users(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].is_admin);
        assert_eq!(
            listed[0].account.as_deref(),
            Some(created.account_slug.as_str())
        );

        assert!(matches!(
            set_admin(&pool, "nobody@example.test", true).await,
            Err(ApiError::NotFound)
        ));
        assert!(matches!(
            set_password(&pool, "nobody@example.test", "long-enough").await,
            Err(ApiError::NotFound)
        ));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn accounts_resolve_by_slug_id_or_email(pool: PgPool) {
        let created = create_user(&pool, new_user("c@example.test", "long-enough", false))
            .await
            .unwrap();
        let id = created.account_id;
        assert_eq!(resolve_account(&pool, &id.to_string()).await.unwrap(), id);
        assert_eq!(
            resolve_account(&pool, &created.account_slug).await.unwrap(),
            id
        );
        assert_eq!(
            resolve_account(&pool, " C@Example.test ").await.unwrap(),
            id
        );
        let unknown_id = Uuid::now_v7().to_string();
        for missing in ["nobody@example.test", "no-such-slug", unknown_id.as_str()] {
            assert!(
                matches!(
                    resolve_account(&pool, missing).await,
                    Err(ApiError::NotFound)
                ),
                "{missing}"
            );
        }
    }
}

#[cfg(test)]
mod password_tests {
    //! Argon2id password hashing: round-trip + cross-version backward
    //! compatibility. Added with the argon2 0.5 → 0.6 upgrade to prove hashes
    //! already stored in `users.password_hash` keep authenticating after the
    //! bump.
    use super::{hash_password, verify_password};
    use chakramcp_shared::error::ApiError;

    // A real Argon2id PHC string produced by argon2 0.5.3 (the pre-upgrade
    // version) for `V05_PASSWORD`. If 0.6 can still verify it, existing users
    // can still log in.
    const V05_PASSWORD: &str = "correct-horse-battery-staple-v05";
    const V05_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$Tq9rqVFKHliQY6aIu/doBg$R2bCUeP9WjSzJ+ylF/JJCqGAzDQc7YbbJhV99iq4CDo";

    #[test]
    fn verifies_hash_produced_by_argon2_0_5() {
        verify_password(V05_PASSWORD, V05_HASH)
            .expect("a stored 0.5-era hash must still verify under argon2 0.6");
        assert!(
            matches!(
                verify_password("wrong", V05_HASH),
                Err(ApiError::Unauthorized)
            ),
            "a wrong password against the 0.5 hash must be rejected"
        );
    }

    #[test]
    fn hash_then_verify_round_trips() {
        let hash = hash_password("s3cr3t-pw").unwrap();
        assert!(
            hash.starts_with("$argon2id$"),
            "unexpected hash format: {hash}"
        );
        verify_password("s3cr3t-pw", &hash).expect("correct password should verify");
        assert!(
            matches!(verify_password("nope", &hash), Err(ApiError::Unauthorized)),
            "incorrect password should be Unauthorized"
        );
    }

    #[test]
    fn each_hash_uses_a_fresh_random_salt() {
        // The 0.6 auto-salt path must still salt per-call: same password,
        // different hashes.
        assert_ne!(
            hash_password("same").unwrap(),
            hash_password("same").unwrap()
        );
    }
}
