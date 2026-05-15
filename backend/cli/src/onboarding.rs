//! First-run wizard for `chakramcp login`.
//!
//! When the user has no network configured, we walk them through:
//!   1. Pick a network (public hosted | local dev | custom URLs)
//!   2. Pick how to sign in (OAuth in browser | paste API key |
//!      device-flow for headless boxes)
//!
//! Subsequent `login` invocations skip the network picker if a network
//! is already chosen — they re-auth the active network unless told
//! otherwise via `--network`.
//!
//! ## Non-interactive use
//!
//! Agents and CI runners that don't have a real TTY hit the picker
//! and fail with `IO error: not a terminal` from dialoguer. To run
//! non-interactively, the caller passes `--method` (browser | api-key
//! | device); if `--method` is absent **and** stdin isn't a TTY we
//! short-circuit with a helpful error pointing at the three
//! non-interactive entry points rather than letting dialoguer panic.

use std::io::IsTerminal;

use anyhow::{bail, Context, Result};

use crate::config::{
    AuthConfig, CliConfig, Network, DEFAULT_NETWORK, LOCAL_APP_URL, LOCAL_RELAY_URL,
    PUBLIC_APP_URL, PUBLIC_RELAY_URL,
};
use crate::{auth, ui};

/// Options forwarded from the `chakramcp login` clap subcommand.
pub struct LoginOptions {
    pub network: Option<String>,
    pub method: Option<Mode>,
    pub api_key: Option<String>,
}

/// Auth method, narrowed down from the clap `LoginMethod` enum.
///
/// `Device` is handled in `main.rs` (it dispatches to `commands::pair`
/// directly), so the wizard only needs to know about Browser /
/// ApiKey / Skip. We keep the `Skip` variant out of the public API —
/// it can only be selected by the interactive picker.
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Browser,
    ApiKey,
}

impl From<crate::LoginMethod> for Mode {
    fn from(m: crate::LoginMethod) -> Self {
        match m {
            crate::LoginMethod::Browser => Mode::Browser,
            crate::LoginMethod::ApiKey => Mode::ApiKey,
            // Device is handled in main.rs before the wizard runs;
            // mapping it to Browser here is unreachable but harmless.
            crate::LoginMethod::Device => Mode::Browser,
        }
    }
}

/// Result of the wizard — the network it left active. The caller is
/// expected to immediately call /v1/me as a confirmation step.
pub struct WizardOutcome {
    pub network: String,
    pub display_account: Option<String>,
}

pub async fn run_login(cfg: &mut CliConfig, opts: LoginOptions) -> Result<WizardOutcome> {
    let stdin_is_tty = std::io::stdin().is_terminal();

    if cfg.networks.is_empty() {
        if !stdin_is_tty {
            bail!(non_interactive_help(
                "no network is configured yet, and the wizard's network picker needs a TTY"
            ));
        }
        ui::banner();
        ui::step("Let's get you connected.");
        let net = pick_network(cfg)?;
        cfg.active = Some(net.clone());
    }

    let target_network = opts
        .network
        .or_else(|| cfg.active.clone())
        .or_else(|| cfg.networks.first().map(|n| n.name.clone()))
        .ok_or_else(|| anyhow::anyhow!("no network available"))?;

    if cfg.network(&target_network).is_none() {
        bail!("no network named '{target_network}' — see `chakramcp networks list`");
    }

    // Resolve the auth method. Explicit `--method` wins; otherwise
    // fall back to either the wizard picker (TTY) or a clear error
    // pointing at the non-interactive options (no TTY).
    let mode = if let Some(m) = opts.method {
        m
    } else if !stdin_is_tty {
        bail!(non_interactive_help(
            "this terminal isn't interactive (no TTY on stdin) so the auth-method picker can't run"
        ));
    } else {
        let needs_fresh_picker = cfg.networks.len() == 1
            && cfg
                .network(&target_network)
                .map(|n| !n.is_signed_in())
                .unwrap_or(true);
        if needs_fresh_picker {
            ui::step("How would you like to sign in?");
            match ui::select(
                "  ↑/↓ to choose, enter to confirm",
                &[
                    "browser    — OAuth (recommended for humans on this device)",
                    "api key    — paste a ck_… key (recommended for headless / CI)",
                    "device     — print a QR + code, approve on another device (RFC 8628)",
                    "skip       — set up later",
                ],
                0,
            )? {
                0 => Mode::Browser,
                1 => Mode::ApiKey,
                2 => {
                    // Dispatch to pair from main.rs would be cleaner,
                    // but the wizard already owns the config mut and
                    // the network choice. Easier to call pair from
                    // inside the wizard with default Args.
                    crate::commands::pair::run(crate::commands::pair::Args::default(), cfg).await?;
                    return Ok(WizardOutcome {
                        network: target_network,
                        display_account: None,
                    });
                }
                _ => {
                    ui::note("Skipped sign-in. Run `chakramcp login` again whenever you're ready.");
                    return Ok(WizardOutcome {
                        network: target_network,
                        display_account: None,
                    });
                }
            }
        } else {
            // Re-login on an existing network: skip the picker, default to OAuth.
            Mode::Browser
        }
    };

    let outcome = match mode {
        Mode::Browser => login_oauth(cfg, &target_network).await?,
        Mode::ApiKey => login_api_key(cfg, &target_network, opts.api_key).await?,
    };

    Ok(outcome)
}

/// Build the helpful three-line error message that surfaces when the
/// wizard can't run interactively. Tailored to the agent / headless
/// case Hermes reported (`IO error: not a terminal`).
fn non_interactive_help(why: &str) -> String {
    format!(
        "{why}.\n\n\
         Pick one of these non-interactive entry points instead:\n\
         \n\
         \x20 chakramcp pair                                  # device-flow (RFC 8628)\n\
         \x20                                                 #   prints a QR URL + 8-char\n\
         \x20                                                 #   code; user approves on\n\
         \x20                                                 #   another device.\n\
         \n\
         \x20 chakramcp configure --api-key ck_…              # paste an existing API key.\n\
         \n\
         \x20 chakramcp login --method browser                # opens (or prints) the OAuth\n\
         \x20                                                 #   URL on the local device.\n\
         \n\
         \x20 chakramcp login --method api-key --api-key ck_… # equivalent to `configure`.\n\
         "
    )
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
            let relay_url = ui::input(
                "Relay service URL",
                Some("https://relay.chakramcp.example.com"),
            )?;
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

async fn login_api_key(
    cfg: &mut CliConfig,
    network: &str,
    provided_key: Option<String>,
) -> Result<WizardOutcome> {
    let key = match provided_key {
        Some(k) => k,
        None => {
            // No key supplied — prompt. ui::password needs a TTY; the
            // caller (run_login) has already gated this path on stdin
            // being a TTY for the non-interactive branch.
            if !std::io::stdin().is_terminal() {
                bail!(non_interactive_help(
                    "--method api-key needs either a key in --api-key or in CHAKRAMCP_API_KEY (stdin isn't a TTY so we can't prompt)"
                ));
            }
            ui::password("API key (ck_…)")?
        }
    };
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
