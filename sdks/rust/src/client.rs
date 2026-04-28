use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

use crate::error::{Error, Result};
use crate::inbox::InboxClient;
use crate::resources::{
    AgentsClient, FriendshipsClient, GrantsClient, InvocationsClient,
};
use crate::types::{
    Agent, Invocation, InvokeRequest, InvokeResponse, MeResponse,
};

pub(crate) const DEFAULT_APP_URL: &str = "https://chakramcp.com";
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
    pub async fn invoke_and_wait(
        &self,
        req: &InvokeRequest,
        opts: PollOpts,
    ) -> Result<Invocation> {
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
        let relay_url =
            trim_url(self.relay_url.unwrap_or_else(|| DEFAULT_RELAY_URL.into()))?;

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
    let body = resp.text().await?;
    if status.is_success() {
        if body.is_empty() {
            return serde_json::from_value(serde_json::Value::Null).map_err(Into::into);
        }
        return serde_json::from_str(&body).map_err(Into::into);
    }
    if let Ok(env) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(obj) = env.get("error").and_then(|e| e.as_object()) {
            return Err(Error::Api {
                status: status.as_u16(),
                code: obj
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                message: obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Err(Error::Api {
        status: status.as_u16(),
        code: "unknown".to_string(),
        message: body,
    })
}

pub(crate) async fn decode_no_body(resp: Response) -> Result<()> {
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status();
    let body = resp.text().await?;
    if let Ok(env) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(obj) = env.get("error").and_then(|e| e.as_object()) {
            return Err(Error::Api {
                status: status.as_u16(),
                code: obj
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                message: obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Err(Error::Api {
        status: status.as_u16(),
        code: "unknown".to_string(),
        message: body,
    })
}
