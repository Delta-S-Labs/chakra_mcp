//! Thin REST client for app + relay services.
//!
//! Authentication is whatever the active network's auth produces — OAuth
//! access_token or API key — both delivered as a Bearer header.

use anyhow::{anyhow, Result};
use reqwest::{Client, Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::{CliConfig, Network};

pub struct ApiClient {
    cfg: CliConfig,
    http: Client,
}

impl ApiClient {
    pub fn new(cfg: CliConfig) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("chakramcp-cli/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self { cfg, http })
    }

    pub fn network(&self) -> Result<&Network> {
        self.cfg.require_active()
    }

    fn bearer(&self) -> Result<String> {
        let net = self.network()?;
        net.bearer().ok_or_else(|| {
            anyhow!(
                "not signed in to network '{}' — run `chakramcp login` or \
                 `chakramcp configure --api-key …`",
                net.name
            )
        })
    }

    fn request(&self, method: Method, base: &str, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", base.trim_end_matches('/'), path);
        self.http.request(method, url)
    }

    pub async fn get_app<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::GET, &net.app_url, path)
            .bearer_auth(bearer)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn put_app<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::PUT, &net.app_url, path)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn post_app<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::POST, &net.app_url, path)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn delete_app<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::DELETE, &net.app_url, path)
            .bearer_auth(bearer)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn get_relay<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::GET, &net.relay_url, path)
            .bearer_auth(bearer)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn post_relay<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::POST, &net.relay_url, path)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn patch_relay<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::PATCH, &net.relay_url, path)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn put_relay<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::PUT, &net.relay_url, path)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn delete_relay<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bearer = self.bearer()?;
        let net = self.network()?;
        let resp = self
            .request(Method::DELETE, &net.relay_url, path)
            .bearer_auth(bearer)
            .send()
            .await?;
        decode(resp).await
    }

    /// GET on the relay where the Bearer is optional — used for the
    /// public discovery endpoint so an operator can browse the
    /// network before signing in.
    pub async fn get_relay_optional_auth<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let net = self.network()?;
        let mut req = self.request(Method::GET, &net.relay_url, path);
        if let Some(bearer) = net.bearer() {
            req = req.bearer_auth(bearer);
        }
        let resp = req.send().await?;
        decode(resp).await
    }
}

/// A refused or failed request: the HTTP status, the server's message, and
/// the trace id to quote when reporting a problem (`ChakraMCP-Trace-Id`,
/// present when the request was traced; round-2 QA N15).
#[derive(Debug)]
pub struct HttpError {
    pub status: StatusCode,
    pub message: String,
    pub trace_id: Option<String>,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.status)?;
        if let Some(trace) = &self.trace_id {
            write!(f, " [trace id {trace}]")?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {}

/// The HTTP status of a failed request, if `e` is one.
pub fn http_status(e: &anyhow::Error) -> Option<StatusCode> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<HttpError>())
        .map(|h| h.status)
}

async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T> {
    let status = resp.status();
    if status == StatusCode::NO_CONTENT {
        let v: serde_json::Value = serde_json::Value::Null;
        return Ok(serde_json::from_value(v)?);
    }
    let trace_id = resp
        .headers()
        .get("chakramcp-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = resp.text().await?;
    if !status.is_success() {
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|env| {
                env.pointer("/error/message")
                    .or_else(|| env.get("error").filter(|e| e.is_string()))
                    .and_then(|m| m.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or(body);
        return Err(HttpError {
            status,
            message,
            trace_id,
        }
        .into());
    }
    serde_json::from_str::<T>(&body)
        .map_err(|e| anyhow!("decoding response failed: {} — body: {}", e, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_request_shows_its_trace_id_and_keeps_its_status() {
        let e: anyhow::Error = HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "boom".into(),
            trace_id: Some("4f12404b".into()),
        }
        .into();
        assert_eq!(
            format!("{e:#}"),
            "boom (500 Internal Server Error) [trace id 4f12404b]"
        );
        // The digits in a trace id don't change how the error is classed.
        assert_eq!(http_status(&e), Some(StatusCode::INTERNAL_SERVER_ERROR));
        let wrapped = e.context("inviting");
        assert_eq!(
            http_status(&wrapped),
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
    }
}
