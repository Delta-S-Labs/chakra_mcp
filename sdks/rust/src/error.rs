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
    /// envelope; when the body isn't one, `message` is the raw body.
    #[error("[{status} {code}] {message}{}", trace_suffix(.trace_id))]
    Api {
        status: u16,
        code: String,
        message: String,
        /// Seconds to wait before retrying, when the server says: the
        /// `Retry-After` header, else the body's `retry_after_secs`.
        /// `None` when it says neither (most errors, and older servers).
        retry_after_secs: Option<u64>,
        /// The request's trace id, from the `ChakraMCP-Trace-Id`
        /// response header: quote it when you report a problem. `None`
        /// when the server didn't trace the request. The error's
        /// `Display` ends with it.
        trace_id: Option<String>,
    },

    /// Public-invoke monthly quota exhausted (HTTP 429, body code
    /// `monthly_quota_exhausted`; migration 0022). Surfaces the
    /// owner-set quota and the UTC instant the window resets so
    /// callers can back off cleanly.
    #[error(
        "[429 monthly_quota_exhausted] {message} (quota={quota}, resets_at={resets_at}){}",
        trace_suffix(.trace_id)
    )]
    QuotaExhausted {
        message: String,
        quota: i32,
        resets_at: chrono::DateTime<chrono::Utc>,
        /// The `Retry-After` delay in seconds, as on [`Error::Api`].
        retry_after_secs: Option<u64>,
        /// The trace id, as on [`Error::Api`].
        trace_id: Option<String>,
    },

    /// The calling account is over its per-minute rate limit (HTTP
    /// 429, body code `account_rate_limited`). `retry_after_secs` is
    /// the rest of the minute (`None` on older servers). The SDK
    /// doesn't retry on its own.
    #[error("[429 account_rate_limited] {message}{}", trace_suffix(.trace_id))]
    RateLimited {
        message: String,
        retry_after_secs: Option<u64>,
        /// The trace id, as on [`Error::Api`].
        trace_id: Option<String>,
    },

    /// The calling account is out of credits (HTTP 429, body code
    /// `account_credits_exhausted`). `retry_after_secs` runs until the
    /// next free monthly grant (`next_grant_at`) and can be weeks long;
    /// buying credits works at once. `credits_url` is where to buy
    /// them, and is `None` on self-hosted servers. All three are `None`
    /// on older servers.
    #[error("[429 account_credits_exhausted] {message}{}", trace_suffix(.trace_id))]
    CreditsExhausted {
        message: String,
        retry_after_secs: Option<u64>,
        next_grant_at: Option<chrono::DateTime<chrono::Utc>>,
        credits_url: Option<String>,
        /// The trace id, as on [`Error::Api`].
        trace_id: Option<String>,
    },

    #[error("invocation timed out after {0:?} - still in flight; check the audit log")]
    InvocationTimeout(std::time::Duration),

    #[error("invocation reached terminal status `{status}` with error `{message:?}`")]
    InvocationFailed {
        status: String,
        message: Option<String>,
    },
}

impl Error {
    /// Seconds to wait before retrying, from whichever API error
    /// variant this is. `None` for other errors, and when the server
    /// sent no delay.
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Error::Api {
                retry_after_secs, ..
            }
            | Error::QuotaExhausted {
                retry_after_secs, ..
            }
            | Error::RateLimited {
                retry_after_secs, ..
            }
            | Error::CreditsExhausted {
                retry_after_secs, ..
            } => *retry_after_secs,
            _ => None,
        }
    }

    /// The request's trace id (the `ChakraMCP-Trace-Id` response
    /// header), from whichever API error variant this is. `None` for
    /// other errors, and when the server didn't trace the request.
    pub fn trace_id(&self) -> Option<&str> {
        match self {
            Error::Api { trace_id, .. }
            | Error::QuotaExhausted { trace_id, .. }
            | Error::RateLimited { trace_id, .. }
            | Error::CreditsExhausted { trace_id, .. } => trace_id.as_deref(),
            _ => None,
        }
    }
}

/// ` (trace id …)` when there is one, for the variants' `Display`.
fn trace_suffix(trace_id: &Option<String>) -> String {
    match trace_id {
        Some(id) => format!(" (trace id {id})"),
        None => String::new(),
    }
}

pub type Result<T> = std::result::Result<T, Error>;
