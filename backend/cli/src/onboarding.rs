//! First-run wizard for `chakramcp login`.
//!
//! When the user has no network configured, we walk them through:
//!   1. Pick a network (public hosted | local dev | custom URLs)
//!   2. Pick how to sign in (OAuth in browser | paste API key)
//!
//! Subsequent `login` invocations skip the network picker if a network
//! is already chosen — they re-auth the active network unless told
//! otherwise via `--network`.

use anyhow::{bail, Context, Result};

use crate::config::{
    AuthConfig, CliConfig, Network, DEFAULT_NETWORK, LOCAL_APP_URL, LOCAL_RELAY_URL,
    PUBLIC_APP_URL, PUBLIC_RELAY_URL,
};
use crate::{auth, ui};

/// Result of the wizard — the network it left active. The caller is
/// expected to immediately call /v1/me as a confirmation step.
pub struct WizardOutcome {
    pub network: String,
    pub display_account: Option<String>,
}

pub async fn run_login(cfg: &mut CliConfig, requested_network: Option<String>) -> Result<WizardOutcome> {
    if cfg.networks.is_empty() {
        ui::banner();
        ui::step("Let's get you connected.");
        let net = pick_network(cfg)?;
        cfg.active = Some(net.clone());
    }

    let target_network = requested_network
        .or_else(|| cfg.active.clone())
        .or_else(|| cfg.networks.first().map(|n| n.name.clone()))
        .ok_or_else(|| anyhow::anyhow!("no network available"))?;

    if cfg.network(&target_network).is_none() {
        bail!("no network named '{target_network}' — see `chakramcp networks list`");
    }

    let mode = if cfg.networks.len() == 1 && cfg.network(&target_network).map(|n| !n.is_signed_in()).unwrap_or(true) {
        // Brand-new flow — ask explicitly.
        ui::step("How would you like to sign in?");
        ui::select(
            "  ↑/↓ to choose, enter to confirm",
            &[
                "browser  — OAuth (recommended for humans)",
                "api key  — paste a ck_… key (recommended for headless use)",
                "skip     — set up later",
            ],
            0,
        )?
    } else {
        // Re-login on an existing network: skip the picker, default to OAuth.
        0
    };

    let outcome = match mode {
        0 => login_oauth(cfg, &target_network).await?,
        1 => login_api_key(cfg, &target_network).await?,
        _ => {
            ui::note("Skipped sign-in. Run `chakramcp login` again whenever you're ready.");
            return Ok(WizardOutcome {
                network: target_network,
                display_account: None,
            });
        }
    };

    Ok(outcome)
}

fn pick_network(cfg: &mut CliConfig) -> Result<String> {
    ui::step("Which network?");
    let choice = ui::select(
        "  ↑/↓ to choose, enter to confirm",
        &[
            "public — the hosted relay at chakramcp.com",
            "local  — http://localhost:8080 + http://localhost:8090 (dev)",
            "custom — paste your own URLs (self-hosted private network)",
        ],
        0,
    )?;
    let net = match choice {
        0 => Network {
            name: "public".to_string(),
            app_url: PUBLIC_APP_URL.into(),
            relay_url: PUBLIC_RELAY_URL.into(),
            oauth_client_id: None,
            auth: AuthConfig::default(),
        },
        1 => Network {
            name: "local".to_string(),
            app_url: LOCAL_APP_URL.into(),
            relay_url: LOCAL_RELAY_URL.into(),
            oauth_client_id: None,
            auth: AuthConfig::default(),
        },
        _ => {
            let name = ui::input("Network name", Some(DEFAULT_NETWORK))?;
            let app_url = ui::input("App service URL", Some("https://chakramcp.example.com"))?;
            let relay_url = ui::input("Relay service URL", Some("https://relay.chakramcp.example.com"))?;
            Network {
                name: name.trim().to_string(),
                app_url: app_url.trim().to_string(),
                relay_url: relay_url.trim().to_string(),
                oauth_client_id: None,
                auth: AuthConfig::default(),
            }
        }
    };
    let name = net.name.clone();
    cfg.add_network(net)?;
    Ok(name)
}

async fn login_oauth(cfg: &mut CliConfig, network: &str) -> Result<WizardOutcome> {
    auth::login(cfg, network).await?;
    let me = me_via_network(cfg, network).await?;
    Ok(WizardOutcome {
        network: network.to_string(),
        display_account: pick_email(&me),
    })
}

async fn login_api_key(cfg: &mut CliConfig, network: &str) -> Result<WizardOutcome> {
    let key = ui::password("API key (ck_…)")?;
    if !key.starts_with("ck_") {
        bail!("API key must start with `ck_`");
    }
    {
        let net = cfg.network_mut(network).unwrap();
        net.auth.api_key = Some(key);
        net.auth.access_token = None;
        net.auth.access_token_expires_at = None;
    }
    cfg.active = Some(network.to_string());
    cfg.save()?;
    let me = me_via_network(cfg, network)
        .await
        .context("API key didn't work — /v1/me rejected it")?;
    Ok(WizardOutcome {
        network: network.to_string(),
        display_account: pick_email(&me),
    })
}

async fn me_via_network(cfg: &CliConfig, network: &str) -> Result<serde_json::Value> {
    let mut clone = cfg.clone();
    clone.active = Some(network.to_string());
    let api = crate::client::ApiClient::new(clone)?;
    api.get_app::<serde_json::Value>("/v1/me").await
}

fn pick_email(me: &serde_json::Value) -> Option<String> {
    me.pointer("/user/email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
