//! Thin REST client for app + relay services.
//!
//! Authentication is whatever the active network's auth produces — OAuth
//! access_token or API key — both delivered as a Bearer header.

use anyhow::{anyhow, bail, Result};
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
}

async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T> {
    let status = resp.status();
    if status == StatusCode::NO_CONTENT {
        let v: serde_json::Value = serde_json::Value::Null;
        return Ok(serde_json::from_value(v)?);
    }
    let body = resp.text().await?;
    if !status.is_success() {
        if let Ok(env) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(msg) = env.pointer("/error/message").and_then(|m| m.as_str()) {
                bail!("{} ({})", msg, status);
            }
        }
        bail!("{}: {}", status, body);
    }
    serde_json::from_str::<T>(&body)
        .map_err(|e| anyhow!("decoding response failed: {} — body: {}", e, body))
}
