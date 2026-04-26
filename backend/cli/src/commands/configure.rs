//! `chakramcp configure --api-key …` — headless / CI sign-in.
//!
//! For first-time users with no networks configured, falls through to
//! the wizard's network picker before storing the key.

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::Value;

use crate::client::ApiClient;
use crate::config::{
    AuthConfig, CliConfig, Network, DEFAULT_NETWORK, LOCAL_APP_URL, LOCAL_RELAY_URL,
    PUBLIC_APP_URL, PUBLIC_RELAY_URL,
};
use crate::ui;

#[derive(Parser, Debug)]
pub struct Args {
    /// The API key (`ck_…`) to save.
    #[arg(long)]
    pub api_key: String,

    /// Network to attach this key to. Defaults to the active network,
    /// or creates a "public" network on first run.
    #[arg(long)]
    pub network: Option<String>,
}

pub async fn run(args: Args, cfg: &mut CliConfig) -> Result<()> {
    if !args.api_key.starts_with("ck_") {
        bail!("API key must start with `ck_`");
    }

    let target = args
        .network
        .or_else(|| cfg.active.clone())
        .unwrap_or_else(|| {
            // No network exists yet — bootstrap a sensible default for headless use.
            // Public hosted is the most common case for CI / agent runtimes.
            "public".to_string()
        });

    if cfg.network(&target).is_none() {
        let (app, relay) = match target.as_str() {
            "public" => (PUBLIC_APP_URL, PUBLIC_RELAY_URL),
            "local" => (LOCAL_APP_URL, LOCAL_RELAY_URL),
            _ => bail!(
                "no network named '{target}' — add it first with `chakramcp networks add`"
            ),
        };
        cfg.add_network(Network {
            name: target.clone(),
            app_url: app.into(),
            relay_url: relay.into(),
            oauth_client_id: None,
            auth: AuthConfig::default(),
        })?;
        ui::note(&format!("created network '{target}' pointing at {app}"));
    }

    // Verify against /v1/me BEFORE persisting, so a bad key doesn't
    // leave stale credentials in the config file.
    let mut probe = cfg.clone();
    {
        let net = probe.network_mut(&target).unwrap();
        net.auth.api_key = Some(args.api_key.clone());
        net.auth.access_token = None;
        net.auth.access_token_expires_at = None;
    }
    probe.active = Some(target.clone());
    let api = ApiClient::new(probe)?;
    let me: Value = api
        .get_app("/v1/me")
        .await
        .context("API key didn't work — /v1/me rejected it")?;

    {
        let net = cfg.network_mut(&target).unwrap();
        net.auth.api_key = Some(args.api_key);
        net.auth.access_token = None;
        net.auth.access_token_expires_at = None;
    }
    cfg.active = Some(target.clone());
    cfg.save()?;

    ui::ok(&format!(
        "configured. signed in as {} on '{target}'",
        me.pointer("/user/email").and_then(|v| v.as_str()).unwrap_or("?")
    ));
    let _ = DEFAULT_NETWORK;
    Ok(())
}
