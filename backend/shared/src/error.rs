use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

/// Common backend error type. Both services use this so error envelopes
/// look identical across services.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("auth error")]
    Auth(#[from] jsonwebtoken::errors::Error),

    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    /// Over the account's per-minute rate limit. Sent with `Retry-After`
    /// (the rest of the window).
    #[error("rate limit exceeded")]
    RateLimited { limit: i32, retry_after_secs: u64 },

    /// Out of credits. Sent with `Retry-After` until the next monthly free
    /// grant (`next_grant_at`); a purchase works at once. `credits_url` is
    /// where to buy or grant credits, on chakramcp.com only.
    #[error("insufficient credits")]
    InsufficientCredits {
        retry_after_secs: u64,
        next_grant_at: chrono::DateTime<chrono::Utc>,
        credits_url: Option<String>,
    },

    /// Public sign-up is closed (`SIGNUP_ENABLED`, or a self-hosted server's
    /// default). The operator creates accounts with `chakramcp-server users`.
    #[error("sign-up is closed on this server: ask its operator for an account")]
    SignupDisabled,

    /// Too many failed sign-ins for one email (the app's `signin_limit`).
    /// Sent with `Retry-After`.
    #[error("too many failed sign-ins for this email: try again in {retry_after_secs} seconds")]
    SigninRateLimited { retry_after_secs: u64 },

    /// An account opened too many credit checkouts in the last hour.
    #[error("too many checkouts for this account in the last hour: try again later")]
    TooManyCheckouts,

    /// The payment provider (Dodo) failed or didn't answer in time.
    #[error("the payment provider couldn't start a checkout: try again")]
    PaymentProvider,

    /// A body the relay must keep whole is too large, e.g. the input to a
    /// capability that asks its owner first (`chk.request.too_large`).
    #[error("{message}")]
    PayloadTooLarge { code: &'static str, message: String },
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    retryable: bool,
    /// When to try again, for usage-limit refusals (also in `Retry-After`,
    /// which browsers may not expose).
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_grant_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_url: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, retryable) = match &self {
            ApiError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, "invalid_request", false),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", false),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", false),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", false),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict", false),
            ApiError::Database(_) | ApiError::Internal(_) => {
                tracing::error!(error = ?self, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", true)
            }
            ApiError::Auth(_) => (StatusCode::UNAUTHORIZED, "unauthorized", false),
            // Per-account usage limits (distinct `code`s from the per-capability
            // public-invoke quota so clients can tell the two apart). Rate is
            // retryable once the 60s window rolls; credits need a top-up or
            // the next monthly grant.
            ApiError::RateLimited { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "account_rate_limited", true)
            }
            ApiError::InsufficientCredits { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "account_credits_exhausted",
                false,
            ),
            ApiError::SignupDisabled => (StatusCode::FORBIDDEN, "signup_disabled", false),
            ApiError::SigninRateLimited { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "signin_rate_limited", true)
            }
            ApiError::TooManyCheckouts => {
                (StatusCode::TOO_MANY_REQUESTS, "too_many_checkouts", true)
            }
            ApiError::PaymentProvider => (StatusCode::BAD_GATEWAY, "payment_provider_error", true),
            ApiError::PayloadTooLarge { code, .. } => (StatusCode::PAYLOAD_TOO_LARGE, *code, false),
        };
        let retry_after = match &self {
            ApiError::SigninRateLimited { retry_after_secs }
            | ApiError::RateLimited {
                retry_after_secs, ..
            }
            | ApiError::InsufficientCredits {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        };
        let (next_grant_at, credits_url) = match &self {
            ApiError::InsufficientCredits {
                next_grant_at,
                credits_url,
                ..
            } => (Some(*next_grant_at), credits_url.clone()),
            _ => (None, None),
        };
        // The sign-in limit keeps its body as it was; the usage limits put
        // the retry hint in the body too.
        let body_retry_after = match &self {
            ApiError::RateLimited { .. } | ApiError::InsufficientCredits { .. } => retry_after,
            _ => None,
        };

        let body = ErrorEnvelope {
            error: ErrorBody {
                code,
                message: self.to_string(),
                retryable,
                retry_after_secs: body_retry_after,
                next_grant_at,
                credits_url,
            },
        };
        let mut response = (status, Json(body)).into_response();
        if let Some(secs) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
