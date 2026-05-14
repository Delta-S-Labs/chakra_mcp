//! `chakramcp-app` — user-facing API service, also reusable as a library
//! so the supervisor binary (`chakramcp-server`) can mount its router
//! into the same process as the relay.

use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub mod auth;
pub mod handlers;
pub mod state;

#[cfg(test)]
pub(crate) mod tests_support;

pub use state::AppState;

/// Mount every public + authenticated route on a fresh router. The
/// caller owns the AppState (database pool + shared config) and is
/// responsible for binding a listener.
pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // ─── Public ────────────────────────────────────
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        // ─── OAuth 2.1 (MCP server auth) ───────────────
        .route(
            "/.well-known/oauth-authorization-server",
            get(handlers::oauth::metadata),
        )
        .route("/oauth/register", post(handlers::oauth::register))
        .route(
            "/oauth/clients/{client_id}",
            get(handlers::oauth::get_client),
        )
        .route("/oauth/issue-code", post(handlers::oauth::issue_code))
        .route("/oauth/token", post(handlers::oauth::token))
        // ─── OAuth 2.1 Device Authorization Grant (RFC 8628)
        .route(
            "/oauth/device_authorization",
            post(handlers::oauth::device_authorization),
        )
        .route(
            "/oauth/device-session/{user_code}",
            get(handlers::oauth::device_session),
        )
        .route(
            "/oauth/device-approve",
            post(handlers::oauth::device_approve),
        )
        .route("/oauth/device-deny", post(handlers::oauth::device_deny))
        // ─── Sign-in callback from frontend ────────────
        .route("/v1/users/upsert", post(handlers::users::upsert))
        // ─── Email + password auth ─────────────────────
        .route("/v1/auth/signup", post(handlers::auth::signup))
        .route("/v1/auth/login", post(handlers::auth::login))
        .route("/v1/auth/signout", post(handlers::auth::signout))
        // ─── Authenticated user routes ─────────────────
        .route("/v1/me", get(handlers::users::me))
        .route(
            "/v1/me/survey",
            get(handlers::surveys::get_mine).post(handlers::surveys::submit),
        )
        .route(
            "/v1/orgs",
            get(handlers::orgs::list).post(handlers::orgs::create),
        )
        .route(
            "/v1/orgs/{slug}",
            get(handlers::orgs::get_one).delete(handlers::orgs::delete_org),
        )
        .route("/v1/orgs/{slug}/members", get(handlers::orgs::list_members))
        .route(
            "/v1/orgs/{slug}/invites",
            post(handlers::orgs::create_invite),
        )
        .route("/v1/invites/{token}", get(handlers::orgs::preview_invite))
        .route(
            "/v1/invites/{token}/accept",
            post(handlers::orgs::accept_invite),
        )
        .route(
            "/v1/api-keys",
            get(handlers::api_keys::list).post(handlers::api_keys::create),
        )
        .route("/v1/api-keys/{id}", delete(handlers::api_keys::revoke))
        .route("/v1/api-keys/{id}/rotate", post(handlers::api_keys::rotate))
        .route("/v1/api-keys/{id}/usage", get(handlers::api_keys::usage))
        // ─── Agent re-parenting ────────────────────────
        .route(
            "/v1/agents/{id}/move-to-personal",
            post(handlers::agents::move_to_personal),
        )
        // ─── Unified paired-sessions view ──────────────
        .route("/v1/pairings", get(handlers::pairings::list))
        .route(
            "/v1/pairings/{kind}/{id}/revoke",
            post(handlers::pairings::revoke),
        )
        // ─── Admin ─────────────────────────────────────
        .route("/v1/admin/users", get(handlers::admin::list_users))
        .route("/v1/admin/orgs", get(handlers::admin::list_orgs))
        .route("/v1/admin/api-keys", get(handlers::admin::list_api_keys))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
