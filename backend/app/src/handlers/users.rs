use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};
use chakramcp_shared::jwt;

use crate::auth::AuthUser;
use crate::state::AppState;

// ─────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpsertRequest {
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub provider_user_id: String,
    /// Raw OAuth profile, stored on oauth_links for later audit/debug.
    pub raw_profile: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct MembershipDto {
    pub account_id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub account_type: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct UpsertResponse {
    pub user: UserDto,
    pub memberships: Vec<MembershipDto>,
    pub token: String,
    /// Whether the network requires this user to fill out the first-login
    /// survey. True only when SURVEY_ENABLED && no completed survey yet.
    pub survey_required: bool,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user: UserDto,
    pub memberships: Vec<MembershipDto>,
    pub survey_required: bool,
}

/// Header carrying the secret shared with the frontend's sign-in callback.
pub const UPSERT_SECRET_HEADER: &str = "x-chakramcp-upsert-secret";

/// Equal-length comparison that doesn't stop at the first differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ─────────────────────────────────────────────────────────
// POST /v1/users/upsert — called by frontend signIn callback
// ─────────────────────────────────────────────────────────
pub async fn upsert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpsertRequest>,
) -> ApiResult<Json<UpsertResponse>> {
    // This trusts the email in the body and returns a session for it, so
    // only the frontend's server-side sign-in callback (which got the email
    // from Google/GitHub) may call it; it proves itself with the shared
    // secret. Unconfigured means no one may.
    let Some(expected) = state.upsert_secret.as_deref() else {
        tracing::error!("UPSERT_SHARED_SECRET is not set: refusing provider sign-in");
        return Err(ApiError::Unauthorized);
    };
    let presented = headers
        .get(UPSERT_SECRET_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        return Err(ApiError::Unauthorized);
    }
    if req.email.trim().is_empty() {
        return Err(ApiError::InvalidRequest("email is required".into()));
    }
    if req.provider.trim().is_empty() || req.provider_user_id.trim().is_empty() {
        return Err(ApiError::InvalidRequest(
            "provider and provider_user_id are required".into(),
        ));
    }

    let admin_email = state.admin_email().map(|s| s.to_lowercase());
    let email_lower = req.email.to_lowercase();
    let is_admin = admin_email.as_deref() == Some(email_lower.as_str());

    let mut tx = state.db.begin().await?;

    // Find user by email (case-insensitive); create if missing.
    let existing_user = sqlx::query!(
        r#"SELECT id, email, display_name, avatar_url, is_admin FROM users WHERE LOWER(email) = $1 LIMIT 1"#,
        email_lower
    )
    .fetch_optional(&mut *tx)
    .await?;

    let (user_id, user_record): (Uuid, UserDto) = match existing_user {
        Some(u) => {
            // Update display_name / avatar / is_admin if they've drifted.
            let updated = sqlx::query!(
                r#"
                UPDATE users
                SET display_name = $2,
                    avatar_url = COALESCE($3, avatar_url),
                    is_admin = $4
                WHERE id = $1
                RETURNING id, email, display_name, avatar_url, is_admin
                "#,
                u.id,
                req.name,
                req.avatar_url,
                is_admin
            )
            .fetch_one(&mut *tx)
            .await?;

            (
                updated.id,
                UserDto {
                    id: updated.id,
                    email: updated.email,
                    display_name: updated.display_name,
                    avatar_url: updated.avatar_url,
                    is_admin: updated.is_admin,
                },
            )
        }
        None => {
            // Create user.
            let new_id = Uuid::now_v7();
            let inserted = sqlx::query!(
                r#"
                INSERT INTO users (id, email, display_name, avatar_url, is_admin)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING id, email, display_name, avatar_url, is_admin
                "#,
                new_id,
                req.email,
                req.name,
                req.avatar_url,
                is_admin
            )
            .fetch_one(&mut *tx)
            .await?;

            // Create their personal account + membership as owner.
            let account_id = Uuid::now_v7();
            let slug = personal_account_slug(&inserted.email);
            sqlx::query!(
                r#"
                INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
                VALUES ($1, $2, $3, 'individual', $4)
                "#,
                account_id,
                slug,
                inserted.display_name,
                inserted.id,
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
                inserted.id,
            )
            .execute(&mut *tx)
            .await?;

            (
                inserted.id,
                UserDto {
                    id: inserted.id,
                    email: inserted.email,
                    display_name: inserted.display_name,
                    avatar_url: inserted.avatar_url,
                    is_admin: inserted.is_admin,
                },
            )
        }
    };

    // Upsert oauth link.
    sqlx::query!(
        r#"
        INSERT INTO oauth_links (id, user_id, provider, provider_user_id, raw_profile)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (provider, provider_user_id) DO UPDATE SET
            raw_profile = EXCLUDED.raw_profile,
            user_id = EXCLUDED.user_id
        "#,
        Uuid::now_v7(),
        user_id,
        req.provider,
        req.provider_user_id,
        req.raw_profile.unwrap_or(serde_json::Value::Null),
    )
    .execute(&mut *tx)
    .await?;

    // Pull memberships.
    let memberships = sqlx::query!(
        r#"
        SELECT a.id as account_id, a.slug, a.display_name, a.account_type, m.role
        FROM account_memberships m
        JOIN accounts a ON a.id = m.account_id
        WHERE m.user_id = $1
        ORDER BY a.account_type DESC, a.created_at ASC
        "#,
        user_id
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| MembershipDto {
        account_id: r.account_id,
        slug: r.slug,
        display_name: r.display_name,
        account_type: r.account_type,
        role: r.role,
    })
    .collect::<Vec<_>>();

    tx.commit().await?;

    let claims = jwt::UserClaims::new(user_id, user_record.email.clone(), user_record.is_admin, 24);
    let token = jwt::encode_jwt(&claims, &state.config.jwt_secret)?;
    let survey_required =
        crate::handlers::surveys::is_required(&state.db, state.config.survey_enabled, user_id)
            .await?;

    Ok(Json(UpsertResponse {
        user: user_record,
        memberships,
        token,
        survey_required,
    }))
}

// ─────────────────────────────────────────────────────────
// GET /v1/me — current user + memberships from JWT
// ─────────────────────────────────────────────────────────
pub async fn me(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<MeResponse>> {
    let row = sqlx::query!(
        r#"SELECT id, email, display_name, avatar_url, is_admin FROM users WHERE id = $1"#,
        user.user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let memberships = sqlx::query!(
        r#"
        SELECT a.id as account_id, a.slug, a.display_name, a.account_type, m.role
        FROM account_memberships m
        JOIN accounts a ON a.id = m.account_id
        WHERE m.user_id = $1
        ORDER BY a.account_type DESC, a.created_at ASC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|r| MembershipDto {
        account_id: r.account_id,
        slug: r.slug,
        display_name: r.display_name,
        account_type: r.account_type,
        role: r.role,
    })
    .collect();

    let survey_required =
        crate::handlers::surveys::is_required(&state.db, state.config.survey_enabled, user.user_id)
            .await?;

    Ok(Json(MeResponse {
        user: UserDto {
            id: row.id,
            email: row.email,
            display_name: row.display_name,
            avatar_url: row.avatar_url,
            is_admin: row.is_admin,
        },
        memberships,
        survey_required,
    }))
}

/// Build a slug for a personal account from an email. Naive — just the
/// part before @, lowercased, with non-slug chars replaced. Collisions
/// are extremely unlikely at our scale; if we hit one, the unique
/// constraint on `accounts.slug` will surface a clear error.
fn personal_account_slug(email: &str) -> String {
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
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use serde_json::json;
    use sqlx::PgPool;
    use tower::ServiceExt;

    use super::UPSERT_SECRET_HEADER;
    use crate::tests_support::test_config;

    const SECRET: &str = "callback-secret-callback-secret-0123456789";

    async fn upsert(
        pool: &PgPool,
        configured: Option<&str>,
        presented: Option<&str>,
    ) -> StatusCode {
        let state = crate::AppState::new(pool.clone(), test_config())
            .with_upsert_secret(configured.map(str::to_owned));
        let mut req = Request::builder()
            .method("POST")
            .uri("/v1/users/upsert")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(secret) = presented {
            req = req.header(UPSERT_SECRET_HEADER, secret);
        }
        let body = json!({
            "email": "someone@example.test",
            "name": "Someone",
            "provider": "github",
            "provider_user_id": "12345",
        });
        crate::router(state)
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
            .status()
    }

    async fn users(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM users WHERE email = 'someone@example.test'")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// The upsert trusts the email it's given, so only the frontend's
    /// sign-in callback, holding the shared secret, may call it.
    #[sqlx::test(migrations = "../migrations")]
    async fn sign_in_needs_the_callback_secret(pool: PgPool) {
        for (configured, presented, why) in [
            (None, Some(SECRET), "no secret configured"),
            (Some(SECRET), None, "no secret presented"),
            (Some(SECRET), Some("wrong"), "wrong secret"),
            (Some(SECRET), Some(&SECRET[1..]), "a prefix of the secret"),
            (Some("   "), Some("   "), "blank counts as unset"),
        ] {
            assert_eq!(
                upsert(&pool, configured, presented).await,
                StatusCode::UNAUTHORIZED,
                "{why}"
            );
        }
        assert_eq!(users(&pool).await, 0, "no session minted, no user created");

        assert_eq!(
            upsert(&pool, Some(SECRET), Some(SECRET)).await,
            StatusCode::OK
        );
        assert_eq!(users(&pool).await, 1);
    }
}
