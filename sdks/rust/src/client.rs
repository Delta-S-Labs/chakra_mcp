use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

use crate::error::{Error, Result};
use crate::inbox::InboxClient;
use crate::resources::{
    AgentsClient, FriendshipsClient, GrantsClient, InvocationsClient, ReviewsClient,
};
use crate::types::{Agent, Invocation, InvokeRequest, InvokeResponse, MeResponse};

pub(crate) const DEFAULT_APP_URL: &str = "https://app.chakramcp.com";
pub(crate) const DEFAULT_RELAY_URL: &str = "https://relay.chakramcp.com";
const USER_AGENT: &str = concat!("chakramcp-rust-sdk/", env!("CARGO_PKG_VERSION"));

/// Top-level client. Cheap to clone - wraps an `Arc`-backed reqwest
/// client internally so handing it to multiple tasks is fine.
#[derive(Clone)]
pub struct ChakraMCP {
    pub(crate) http: Client,
    pub(crate) app_url: String,
    pub(crate) relay_url: String,
}

impl ChakraMCP {
    /// Construct a client with the hosted-network defaults.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::builder().api_key(api_key).build()
    }

    pub fn builder() -> ChakraMCPBuilder {
        ChakraMCPBuilder::default()
    }

    pub fn app_url(&self) -> &str {
        &self.app_url
    }
    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    /// Sub-client constructors. Free to call repeatedly - they wrap
    /// the parent by reference.
    pub fn agents(&self) -> AgentsClient<'_> {
        AgentsClient::new(self)
    }
    pub fn friendships(&self) -> FriendshipsClient<'_> {
        FriendshipsClient::new(self)
    }
    pub fn grants(&self) -> GrantsClient<'_> {
        GrantsClient::new(self)
    }
    pub fn invocations(&self) -> InvocationsClient<'_> {
        InvocationsClient::new(self)
    }
    pub fn inbox(&self) -> InboxClient<'_> {
        InboxClient::new(self)
    }
    pub fn reviews(&self) -> ReviewsClient<'_> {
        ReviewsClient::new(self)
    }

    // ─── Top-level RPC ───────────────────────────────────

    pub async fn me(&self) -> Result<MeResponse> {
        self.app_get("/v1/me").await
    }

    pub async fn network(&self) -> Result<Vec<Agent>> {
        self.relay_get("/v1/network/agents").await
    }

    pub async fn invoke(&self, req: &InvokeRequest) -> Result<InvokeResponse> {
        self.relay_post("/v1/invoke", req).await
    }

    /// Enqueue + poll until terminal status. Default: poll every
    /// 1500ms, time out after 3 minutes.
    pub async fn invoke_and_wait(&self, req: &InvokeRequest, opts: PollOpts) -> Result<Invocation> {
        let interval = opts.interval.unwrap_or(Duration::from_millis(1500));
        let timeout = opts.timeout.unwrap_or(Duration::from_secs(180));
        let started = std::time::Instant::now();

        let enq = self.invoke(req).await?;
        if enq.status.is_terminal() {
            return self.invocations().get(&enq.invocation_id).await;
        }

        loop {
            if started.elapsed() >= timeout {
                return Err(Error::InvocationTimeout(timeout));
            }
            tokio::time::sleep(interval).await;
            let fresh = self.invocations().get(&enq.invocation_id).await?;
            if fresh.status.is_terminal() {
                return Ok(fresh);
            }
        }
    }

    // ─── Internal request plumbing ────────────────────────

    pub(crate) fn app_request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.app_url, path))
    }
    pub(crate) fn relay_request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.relay_url, path))
    }

    pub(crate) async fn app_get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        decode(self.app_request(Method::GET, path).send().await?).await
    }
    pub(crate) async fn relay_get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        decode(self.relay_request(Method::GET, path).send().await?).await
    }
    pub(crate) async fn relay_post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        decode(
            self.relay_request(Method::POST, path)
                .json(body)
                .send()
                .await?,
        )
        .await
    }
    pub(crate) async fn relay_patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        decode(
            self.relay_request(Method::PATCH, path)
                .json(body)
                .send()
                .await?,
        )
        .await
    }
    pub(crate) async fn relay_delete(&self, path: &str) -> Result<()> {
        let resp = self.relay_request(Method::DELETE, path).send().await?;
        decode_no_body(resp).await
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PollOpts {
    pub interval: Option<Duration>,
    pub timeout: Option<Duration>,
}

#[derive(Default)]
pub struct ChakraMCPBuilder {
    api_key: Option<String>,
    app_url: Option<String>,
    relay_url: Option<String>,
    request_timeout: Option<Duration>,
    custom_http: Option<Client>,
}

impl ChakraMCPBuilder {
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
    pub fn app_url(mut self, url: impl Into<String>) -> Self {
        self.app_url = Some(url.into());
        self
    }
    pub fn relay_url(mut self, url: impl Into<String>) -> Self {
        self.relay_url = Some(url.into());
        self
    }
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }
    pub fn http_client(mut self, client: Client) -> Self {
        self.custom_http = Some(client);
        self
    }

    pub fn build(self) -> Result<ChakraMCP> {
        let api_key = self.api_key.ok_or(Error::InvalidApiKey)?;
        if !api_key.starts_with("ck_") {
            return Err(Error::InvalidApiKey);
        }

        let app_url = trim_url(self.app_url.unwrap_or_else(|| DEFAULT_APP_URL.into()))?;
        let relay_url = trim_url(self.relay_url.unwrap_or_else(|| DEFAULT_RELAY_URL.into()))?;

        let http = if let Some(c) = self.custom_http {
            c
        } else {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}"))
                    .map_err(|e| Error::InvalidUrl(e.to_string()))?,
            );
            headers.insert(
                reqwest::header::USER_AGENT,
                reqwest::header::HeaderValue::from_static(USER_AGENT),
            );
            Client::builder()
                .default_headers(headers)
                .timeout(self.request_timeout.unwrap_or(Duration::from_secs(60)))
                .build()?
        };

        Ok(ChakraMCP {
            http,
            app_url,
            relay_url,
        })
    }
}

fn trim_url(url: String) -> Result<String> {
    let u = url::Url::parse(&url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
    Ok(u.as_str().trim_end_matches('/').to_string())
}

pub(crate) async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T> {
    if resp.status() == StatusCode::NO_CONTENT {
        // T might be Option-like or () - try to deserialize from null.
        return serde_json::from_value(serde_json::Value::Null).map_err(Into::into);
    }
    let status = resp.status();
    let retry_after = retry_after_header(&resp);
    let body = resp.text().await?;
    if status.is_success() {
        if body.is_empty() {
            return serde_json::from_value(serde_json::Value::Null).map_err(Into::into);
        }
        return serde_json::from_str(&body).map_err(Into::into);
    }
    Err(api_error(status, retry_after, body))
}

pub(crate) async fn decode_no_body(resp: Response) -> Result<()> {
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status();
    let retry_after = retry_after_header(&resp);
    let body = resp.text().await?;
    Err(api_error(status, retry_after, body))
}

/// The `Retry-After` header, when it's a whole number of seconds.
fn retry_after_header(resp: &Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Turn a non-2xx response into an [`Error`]. `retry_after` is the
/// `Retry-After` header; the body's `retry_after_secs` stands in when
/// it's missing.
fn api_error(status: StatusCode, retry_after: Option<u64>, body: String) -> Error {
    if let Ok(env) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(obj) = env.get("error").and_then(|e| e.as_object()) {
            let retry_after_secs =
                retry_after.or_else(|| obj.get("retry_after_secs").and_then(|v| v.as_u64()));
            if let Some(err) = decode_usage_limit(status, &env, obj, retry_after_secs) {
                return err;
            }
            return Error::Api {
                status: status.as_u16(),
                code: str_field(obj, "code").unwrap_or("unknown").to_string(),
                message: str_field(obj, "message").unwrap_or("").to_string(),
                retry_after_secs,
            };
        }
    }
    Error::Api {
        status: status.as_u16(),
        code: "unknown".to_string(),
        message: body,
        retry_after_secs: retry_after,
    }
}

/// Surface the usage-limit 429s as typed variants rather than the
/// generic `Error::Api`, so callers can pattern-match on them for
/// back-off without parsing the envelope themselves.
fn decode_usage_limit(
    status: StatusCode,
    env: &serde_json::Value,
    err_obj: &serde_json::Map<String, serde_json::Value>,
    retry_after_secs: Option<u64>,
) -> Option<Error> {
    if status != StatusCode::TOO_MANY_REQUESTS {
        return None;
    }
    let message = str_field(err_obj, "message").unwrap_or("").to_string();
    match str_field(err_obj, "code")? {
        // Public-invoke monthly quota on the target capability.
        "monthly_quota_exhausted" => {
            let quota = env.get("quota")?.as_i64()? as i32;
            let resets_at = parse_utc(env.get("resets_at")?.as_str()?)?;
            Some(Error::QuotaExhausted {
                message,
                quota,
                resets_at,
                retry_after_secs,
            })
        }
        // The calling account's own limits.
        "account_rate_limited" => Some(Error::RateLimited {
            message,
            retry_after_secs,
        }),
        "account_credits_exhausted" => Some(Error::CreditsExhausted {
            message,
            retry_after_secs,
            next_grant_at: str_field(err_obj, "next_grant_at").and_then(parse_utc),
            credits_url: str_field(err_obj, "credits_url").map(str::to_string),
        }),
        _ => None,
    }
}

fn str_field<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    obj.get(key).and_then(|v| v.as_str())
}

fn parse_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}
