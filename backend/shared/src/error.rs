use axum::http::StatusCode;
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
        };

        let body = ErrorEnvelope {
            error: ErrorBody {
                code,
                message: self.to_string(),
                retryable,
            },
        };
        (status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
