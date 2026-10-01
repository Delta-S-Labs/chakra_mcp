//! The sign-in pages a self-hosted server serves itself (spec §6 of
//! `docs/superpowers/specs/2026-10-01-self-hosting-phase3-design.md`):
//!
//! - `/oauth/authorize`: sign in, then consent, for `chakramcp login` and
//!   MCP clients;
//! - `/signup`, while sign-up is open;
//! - `/app/pair` and `/qr`: approving a device pairing, and its QR handoff;
//! - `POST /logout`.
//!
//! They sit on the same paths as chakramcp.com's web UI, so the discovery
//! document and the device flow need no change, and they're mounted only
//! when `HOSTING_MODE=self_hosted`. Plain HTML forms with no JavaScript;
//! every step calls the same functions as the HTTP API (`accounts`,
//! `handlers::oauth`), so the two can't drift.
//!
//! Security: a session cookie that isn't an API token (`session`), CSRF
//! tokens plus an `Origin` check on every form post (`csrf`), and strict
//! response headers, same-origin `return_to` and `/qr` (`security`).

mod authorize;
mod csrf;
mod pair;
mod qr;
mod security;
mod session;
mod signup;
mod views;

use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use chakramcp_shared::error::ApiError;

use crate::state::AppState;
use security::{message, AppOrigin};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/oauth/authorize",
            get(authorize::show).post(authorize::submit),
        )
        .route("/signup", get(signup::show).post(signup::submit))
        .route("/app/pair", get(pair::show).post(pair::submit))
        .route("/qr", get(qr::show))
        .route("/logout", post(session::logout))
        .route("/assets/pages.css", get(security::stylesheet))
}

/// A form whose CSRF token or origin didn't check out: usually a page left
/// open, or a cookie the browser dropped.
fn expired_form(origin: &AppOrigin) -> Response {
    message(
        origin,
        StatusCode::FORBIDDEN,
        "This form expired",
        "Go back, reload the page and try again.",
        None,
    )
}

fn server_error(origin: &AppOrigin, error: impl std::fmt::Display) -> Response {
    tracing::error!(error = %error, "a sign-in page step failed");
    message(
        origin,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Something went wrong",
        "The server couldn't finish that step. Try again in a moment.",
        None,
    )
}

/// What a sign-in form says when the password check fails, and its status.
fn signin_failure(error: ApiError) -> (StatusCode, String) {
    match error {
        ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Wrong email or password.".into()),
        ApiError::SigninRateLimited { retry_after_secs } => (
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Too many failed attempts for this email. Try again in {} minutes.",
                retry_after_secs.div_ceil(60)
            ),
        ),
        other => {
            tracing::error!(error = %other, "sign-in failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Signing in failed. Try again in a moment.".into(),
            )
        }
    }
}

#[cfg(test)]
mod tests;
