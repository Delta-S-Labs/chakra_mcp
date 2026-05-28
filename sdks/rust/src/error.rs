use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid API key - must start with `ck_`")]
    InvalidApiKey,

    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The relay/app responded with a non-2xx status. `code` and
    /// `message` come from the standard `{"error":{"code","message"}}`
    /// envelope; `body` is the raw response body for debugging.
    #[error("[{status} {code}] {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },

    /// Public-invoke monthly quota exhausted (HTTP 429, body code
    /// `monthly_quota_exhausted`; migration 0022). Surfaces the
    /// owner-set quota and the UTC instant the window resets so
    /// callers can back off cleanly.
    #[error("[429 monthly_quota_exhausted] {message} (quota={quota}, resets_at={resets_at})")]
    QuotaExhausted {
        message: String,
        quota: i32,
        resets_at: chrono::DateTime<chrono::Utc>,
    },

    #[error("invocation timed out after {0:?} - still in flight; check the audit log")]
    InvocationTimeout(std::time::Duration),

    #[error("invocation reached terminal status `{status}` with error `{message:?}`")]
    InvocationFailed {
        status: String,
        message: Option<String>,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
