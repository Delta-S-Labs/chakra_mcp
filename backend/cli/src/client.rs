//! Thin REST client for app + relay services.
//!
//! Authentication is whatever the loaded CliConfig produces — OAuth
//! access_token or API key — both delivered as a Bearer header.

use anyhow::{anyhow, bail, Result};
use reqwest::{Client, Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::CliConfig;

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

    pub fn config(&self) -> &CliConfig {
        &self.cfg
    }

    fn bearer(&self) -> Result<String> {
        self.cfg.bearer().ok_or_else(|| {
            anyhow!("not signed in — run `chakramcp login` or `chakramcp configure --api-key …`")
        })
    }

    fn request(&self, method: Method, base: &str, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", base.trim_end_matches('/'), path);
        self.http.request(method, url)
    }

    pub fn app_url(&self) -> &str {
        &self.cfg.server.app_url
    }
    pub fn relay_url(&self) -> &str {
        &self.cfg.server.relay_url
    }

    pub async fn get_app<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bearer = self.bearer()?;
        let resp = self
            .request(Method::GET, self.app_url(), path)
            .bearer_auth(bearer)
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn get_relay<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bearer = self.bearer()?;
        let resp = self
            .request(Method::GET, self.relay_url(), path)
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
        let resp = self
            .request(Method::POST, self.relay_url(), path)
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
        // Return Unit if T is (); otherwise this will fail at deserialize and that's fine.
        let v: serde_json::Value = serde_json::Value::Null;
        return Ok(serde_json::from_value(v)?);
    }
    let body = resp.text().await?;
    if !status.is_success() {
        // Try to surface a structured error envelope.
        if let Ok(env) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(msg) = env
                .pointer("/error/message")
                .and_then(|m| m.as_str())
            {
                bail!("{} ({})", msg, status);
            }
        }
        bail!("{}: {}", status, body);
    }
    serde_json::from_str::<T>(&body)
        .map_err(|e| anyhow!("decoding response failed: {} — body: {}", e, body))
}
