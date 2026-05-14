//! Email + password authentication handlers.
//!
//! Two endpoints:
//!   POST /v1/auth/signup   email + password + name → user + JWT
//!   POST /v1/auth/login    email + password         → user + JWT
//!
//! Passwords are hashed with Argon2id. The `users.password_hash` column
//! stores the full PHC string (including salt + parameters). We never
//! store plaintext.
//!
//! TODO (separate slice):
//!   * Email verification — send a magic link, set users.email_verified_at
//!   * Password reset — token-based reset flow
//!   * Rate limiting per IP / email
//!   * Lockout after N failed attempts

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};
use chakramcp_shared::jwt;

use crate::auth::AuthUser;
use crate::handlers::users::{MembershipDto, UserDto};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub user: UserDto,
    pub memberships: Vec<MembershipDto>,
    pub token: String,
    pub survey_required: bool,
}

const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 200;

// ─────────────────────────────────────────────────────────
// POST /v1/auth/signup
// ─────────────────────────────────────────────────────────
pub async fn signup(
    State(state): State<AppState>,
    Json(req): Json<SignupRequest>,
) -> ApiResult<Json<AuthResponse>> {
    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(ApiError::InvalidRequest("a valid email is required".into()));
    }
    if req.password.len() < MIN_PASSWORD_LEN || req.password.len() > MAX_PASSWORD_LEN {
        return Err(ApiError::InvalidRequest(format!(
            "password must be {MIN_PASSWORD_LEN}–{MAX_PASSWORD_LEN} characters"
        )));
    }
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }

    let exists = sqlx::query!(
        r#"SELECT id FROM users WHERE LOWER(email) = $1 LIMIT 1"#,
        email
    )
    .fetch_optional(&state.db)
    .await?;
    if exists.is_some() {
        return Err(ApiError::Conflict(
            "an account with this email already exists".into(),
        ));
    }

    let password_hash = hash_password(&req.password)?;
    let admin_email = state.admin_email().map(|s| s.to_lowercase());
    let is_admin = admin_email.as_deref() == Some(email.as_str());

    let mut tx = state.db.begin().await?;

    let user_id = Uuid::now_v7();
    let user = sqlx::query!(
        r#"
        INSERT INTO users (id, email, display_name, is_admin, password_hash)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, email, display_name, avatar_url, is_admin
        "#,
        user_id,
        req.email,
        name,
        is_admin,
        password_hash,
    )
    .fetch_one(&mut *tx)
    .await?;

    // Personal account + owner membership.
    let account_id = Uuid::now_v7();
    let slug = personal_slug(&user.email);
    sqlx::query!(
        r#"
        INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
        VALUES ($1, $2, $3, 'individual', $4)
        "#,
        account_id,
        slug,
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

    let memberships = load_memberships(&state.db, user.id).await?;
    let user_dto = UserDto {
        id: user.id,
        email: user.email.clone(),
        display_name: user.display_name,
        avatar_url: user.avatar_url,
        is_admin: user.is_admin,
    };
    let claims = jwt::UserClaims::new(user.id, user.email, user.is_admin, 24);
    let token = jwt::encode_jwt(&claims, &state.config.jwt_secret)?;
    let survey_required =
        crate::handlers::surveys::is_required(&state.db, state.config.survey_enabled, user_dto.id)
            .await?;

    Ok(Json(AuthResponse {
        user: user_dto,
        memberships,
        token,
        survey_required,
    }))
}

// ─────────────────────────────────────────────────────────
// POST /v1/auth/login
// ─────────────────────────────────────────────────────────
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Json<AuthResponse>> {
    let email = req.email.trim().to_lowercase();

    let row = sqlx::query!(
        r#"
        SELECT id, email, display_name, avatar_url, is_admin, password_hash
        FROM users
        WHERE LOWER(email) = $1
        LIMIT 1
        "#,
        email
    )
    .fetch_optional(&state.db)
    .await?;

    // Same generic error for "no such user" and "wrong password" — we
    // don't want to leak which emails exist on the network.
    let row = match row {
        Some(r) if r.password_hash.is_some() => r,
        _ => return Err(ApiError::Unauthorized),
    };
    let stored_hash = row.password_hash.as_ref().unwrap();

    verify_password(&req.password, stored_hash)?;

    let memberships = load_memberships(&state.db, row.id).await?;
    let user_dto = UserDto {
        id: row.id,
        email: row.email.clone(),
        display_name: row.display_name,
        avatar_url: row.avatar_url,
        is_admin: row.is_admin,
    };
    let claims = jwt::UserClaims::new(row.id, row.email, row.is_admin, 24);
    let token = jwt::encode_jwt(&claims, &state.config.jwt_secret)?;
    let survey_required =
        crate::handlers::surveys::is_required(&state.db, state.config.survey_enabled, user_dto.id)
            .await?;

    Ok(Json(AuthResponse {
        user: user_dto,
        memberships,
        token,
        survey_required,
    }))
}

// ─────────────────────────────────────────────────────────
// POST /v1/auth/signout
// ─────────────────────────────────────────────────────────
//
// Records the caller's JWT in `revoked_tokens` so it can no longer
// authenticate, even if the plaintext was previously exfiltrated.
// Clearing the NextAuth cookie alone isn't enough — a stolen JWT
// lives until its 24h `exp` unless we kill it here.
//
// The `AuthUser` extractor already ran the revocation check, so a
// token can't be used to sign itself out twice (and an already-revoked
// token gets the same 401 every other request gets).
//
// Idempotent on the `jti` PK — repeat calls before the cookie clear
// commits are a no-op via ON CONFLICT.
pub async fn signout(State(state): State<AppState>, user: AuthUser) -> ApiResult<StatusCode> {
    // API-key requests don't carry a jti and have their own revocation
    // path (`api_keys.revoked_at`). Treat them as a no-op rather than
    // an error so the frontend doesn't have to special-case which
    // credential the session is using.
    let (Some(jti), Some(expires_at)) = (user.jti, user.token_expires_at) else {
        return Ok(StatusCode::NO_CONTENT);
    };

    sqlx::query!(
        r#"
        INSERT INTO revoked_tokens (jti, user_id, expires_at, reason)
        VALUES ($1, $2, $3, 'user_signout')
        ON CONFLICT (jti) DO NOTHING
        "#,
        jti,
        user.user_id,
        expires_at,
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────

fn hash_password(plain: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("password hashing failed: {e}")))?;
    Ok(hash.to_string())
}

fn verify_password(plain: &str, stored: &str) -> Result<(), ApiError> {
    let parsed = PasswordHash::new(stored)
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("stored password hash is malformed")))?;
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .map_err(|_| ApiError::Unauthorized)
}

async fn load_memberships(
    db: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<MembershipDto>, ApiError> {
    let rows = sqlx::query!(
        r#"
        SELECT a.id as account_id, a.slug, a.display_name, a.account_type, m.role
        FROM account_memberships m
        JOIN accounts a ON a.id = m.account_id
        WHERE m.user_id = $1
        ORDER BY a.account_type DESC, a.created_at ASC
        "#,
        user_id
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| MembershipDto {
            account_id: r.account_id,
            slug: r.slug,
            display_name: r.display_name,
            account_type: r.account_type,
            role: r.role,
        })
        .collect())
}

fn personal_slug(email: &str) -> String {
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
mod signout_tests {
    //! End-to-end test of the revocation flow:
    //!
    //!   1. Mint a JWT for a fresh user.
    //!   2. Hit a protected endpoint (`GET /v1/me`) → 200.
    //!   3. POST /v1/auth/signout with the same JWT → 204.
    //!   4. Re-hit the protected endpoint with the SAME JWT → 401.
    //!
    //! Also covers the idempotency path (sign-out twice → second call
    //! is the same 401 the protected endpoint gives, since the
    //! extractor refuses revoked tokens before the handler runs).

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use chakramcp_shared::jwt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    const TEST_SECRET: &str = "test-secret-test-secret-test-secret-test-secret";

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: TEST_SECRET.into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    async fn seed_user_and_token(pool: &PgPool) -> String {
        let user_id = Uuid::now_v7();
        let email = format!("{user_id}@t.local");
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Test User', 'x')"#,
            user_id,
            email,
        )
        .execute(pool)
        .await
        .unwrap();
        let claims = jwt::UserClaims::new(user_id, email, false, 1);
        jwt::encode_jwt(&claims, TEST_SECRET).unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn signout_revokes_token_and_kills_future_requests(pool: PgPool) {
        let token = seed_user_and_token(&pool).await;
        let state = crate::AppState::new(pool, config());

        // (1 + 2) /v1/me with a fresh token → 200.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "fresh JWT should be accepted by /v1/me"
        );

        // (3) /v1/auth/signout → 204.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/signout")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::NO_CONTENT,
            "sign-out with a valid token should return 204",
        );

        // (4) Same JWT against /v1/me → 401. This is the whole point of
        // the slice — exfiltrated tokens stop working at sign-out.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "a revoked JWT must not authenticate further requests",
        );

        // (5) Re-signing-out should now itself 401 — the extractor's
        // revocation check fires before the handler runs.
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/signout")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "double sign-out should be rejected by the extractor",
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn unrevoked_token_continues_to_work(pool: PgPool) {
        // Sanity check that the revocation check is targeted — a
        // *different* user's sign-out must not affect this user's
        // token.
        let alice_token = seed_user_and_token(&pool).await;
        let bob_token = seed_user_and_token(&pool).await;
        let state = crate::AppState::new(pool, config());

        // Bob signs out.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/signout")
                    .header(header::AUTHORIZATION, format!("Bearer {bob_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // Alice is still authenticated.
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "another user's revocation must not invalidate this user's token",
        );
    }
}
