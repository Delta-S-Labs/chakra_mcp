//! CSRF protection for the pages' forms: a random token in a
//! `SameSite=Strict` cookie, mirrored in a hidden field, compared in
//! constant time on every post. When the browser sends an `Origin` header
//! it must be this server's; `Origin: null` is refused. Without one (curl,
//! some older clients), the token decides.

use axum::http::{header, HeaderMap};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use subtle::ConstantTimeEq;

use super::security::AppOrigin;

const COOKIE: &str = "chakramcp_csrf";

/// The token for this response's forms: the browser's, or a new one, added
/// to the jar.
pub(crate) fn token(jar: CookieJar, origin: &AppOrigin) -> (CookieJar, String) {
    if let Some(existing) = jar.get(COOKIE).map(|c| c.value().to_owned()) {
        if existing.len() >= 32 {
            return (jar, existing);
        }
    }
    let token = crate::handlers::oauth::random_token(32);
    let cookie = Cookie::build((COOKIE, token.clone()))
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(origin.secure())
        .path("/")
        .build();
    (jar.add(cookie), token)
}

/// Whether a form post is one of this server's own.
pub(crate) fn verify(
    jar: &CookieJar,
    headers: &HeaderMap,
    form_token: &str,
    origin: &AppOrigin,
) -> bool {
    if let Some(sent) = headers.get(header::ORIGIN) {
        if sent.as_bytes() != origin.origin().as_bytes() {
            return false;
        }
    }
    let Some(cookie) = jar.get(COOKIE) else {
        return false;
    };
    let expected = cookie.value().as_bytes();
    !form_token.is_empty()
        && expected.len() == form_token.len()
        && bool::from(expected.ct_eq(form_token.as_bytes()))
}
