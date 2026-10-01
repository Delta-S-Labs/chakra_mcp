//! The pages' sign-in session: a JWT in an `HttpOnly`, `SameSite=Lax`
//! cookie, signed with a key derived from `JWT_SECRET` so it is *not* an API
//! token (the API, which verifies with `JWT_SECRET` itself, refuses it).
//! Non-admin, one hour, and revoked through `revoked_tokens` on sign-out.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use axum_extra::extract::Form;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use uuid::Uuid;

use chakramcp_shared::error::ApiResult;
use chakramcp_shared::jwt;

use super::csrf;
use super::security::{message, AppOrigin};
use crate::accounts::SignedIn;
use crate::state::AppState;

const COOKIE: &str = "chakramcp_page_session";
const TTL_HOURS: i64 = 1;
const KEY_LABEL: &[u8] = b"chakramcp-pages-v1";

/// Who is signed in on the pages.
#[derive(Debug, Clone)]
pub(crate) struct PageUser {
    pub user_id: Uuid,
    pub email: String,
    jti: Uuid,
    exp: i64,
}

fn key(jwt_secret: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(jwt_secret.as_bytes())
        .expect("HMAC takes any key");
    mac.update(KEY_LABEL);
    hex::encode(mac.finalize().into_bytes())
}

/// Sign `user` in on the pages.
pub(crate) fn start(
    state: &AppState,
    jar: CookieJar,
    user: &SignedIn,
    origin: &AppOrigin,
) -> ApiResult<CookieJar> {
    let claims = jwt::UserClaims::new(user.user_id, user.email.clone(), false, TTL_HOURS);
    let token = jwt::encode_jwt(&claims, &key(&state.config.jwt_secret))?;
    let cookie = Cookie::build((COOKIE, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(origin.secure())
        .path("/")
        .build();
    Ok(jar.add(cookie))
}

/// The signed-in user, if the cookie holds a live session for a user who
/// still exists.
pub(crate) async fn current(state: &AppState, jar: &CookieJar) -> Option<PageUser> {
    let token = jar.get(COOKIE)?.value().to_owned();
    let claims = jwt::decode_jwt(&token, &key(&state.config.jwt_secret)).ok()?;
    if crate::auth::is_token_revoked(&state.db, claims.jti)
        .await
        .unwrap_or(true)
    {
        return None;
    }
    let email = sqlx::query_scalar!("SELECT email FROM users WHERE id = $1", claims.sub)
        .fetch_optional(&state.db)
        .await
        .ok()??;
    Some(PageUser {
        user_id: claims.sub,
        email,
        jti: claims.jti,
        exp: claims.exp,
    })
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LogoutForm {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    return_to: String,
}

/// `POST /logout`: revoke the session and clear the cookie.
pub(crate) async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(form): Form<LogoutForm>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !csrf::verify(&jar, &headers, &form.csrf, &origin) {
        return super::expired_form(&origin);
    }
    if let Some(user) = current(&state, &jar).await {
        let expires_at = Utc
            .timestamp_opt(user.exp, 0)
            .single()
            .unwrap_or_else(Utc::now);
        let revoked = sqlx::query!(
            r#"
            INSERT INTO revoked_tokens (jti, user_id, expires_at, reason)
            VALUES ($1, $2, $3, 'page_signout')
            ON CONFLICT (jti) DO NOTHING
            "#,
            user.jti,
            user.user_id,
            expires_at,
        )
        .execute(&state.db)
        .await;
        if let Err(e) = revoked {
            tracing::error!(error = %e, "couldn't revoke a page session");
            return message(
                &origin,
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "Signing out failed. Try again.",
                None,
            );
        }
    }
    let jar = jar.remove(Cookie::build(COOKIE).path("/"));
    match origin.return_path(&form.return_to) {
        Some(path) => (jar, Redirect::to(&path)).into_response(),
        None => (
            jar,
            message(
                &origin,
                StatusCode::OK,
                "Signed out",
                "You're signed out of this server's pages.",
                None,
            ),
        )
            .into_response(),
    }
}
