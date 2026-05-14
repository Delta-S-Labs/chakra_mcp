//! `chakramcp pair` — device-flow pairing (RFC 8628).
//!
//! Complements the two existing auth paths:
//!
//! * `chakramcp login` — OAuth 2.1 + PKCE, opens a browser on the same
//!   device, returns a token.
//! * `chakramcp configure --api-key ck_…` — headless / CI.
//! * `chakramcp pair` — what this file is. Agent and user can be on
//!   different devices. Prints a clickable URL, a hosted-QR URL (so the
//!   user can scan from their phone), and an 8-char user_code. Polls
//!   `/oauth/token` until the user approves on `/app/pair`, then saves
//!   the resulting Bearer JWT into the active network's config.
//!
//! The endpoint shape matches the device-authorization server-side
//! handler in `backend/app/src/handlers/oauth.rs` — keep them in sync.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use clap::Args as ClapArgs;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::config::CliConfig;
use crate::ui;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Suggest a persona to the relay (cosmetic, hints the consent UI).
    /// Common: `hermes`, `openclaw`. Free text.
    #[arg(long)]
    pub persona: Option<String>,

    /// Suggest an agent slug to pre-fill on the consent page. User can
    /// override at approval time.
    #[arg(long)]
    pub agent_slug: Option<String>,

    /// Suggest a display name (cosmetic).
    #[arg(long)]
    pub display_name: Option<String>,

    /// Suggest a description (cosmetic).
    #[arg(long)]
    pub description: Option<String>,

    /// Skip auto-opening the QR URL in the local browser. Useful on
    /// headless boxes — the URLs still get printed for hand-paste.
    #[arg(long)]
    pub no_open: bool,
}

#[derive(Debug, Serialize)]
struct DeviceAuthRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    persona: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_slug_hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_display_name_hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_description_hint: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    verification_uri_qr: String,
    #[allow(dead_code)]
    expires_in: i64,
    interval: i32,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    #[allow(dead_code)]
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_slug: Option<String>,
    #[serde(default)]
    account_slug: Option<String>,
}

/// RFC 8628 §3.5 error codes the token endpoint may return during polling.
/// Anything else (e.g. `invalid_grant` on a consumed/expired code) is fatal.
#[derive(Debug, Deserialize)]
struct PollError {
    error: String,
}

pub async fn run(args: Args, cfg: &mut CliConfig) -> Result<()> {
    let network_name = cfg
        .active
        .clone()
        .ok_or_else(|| anyhow!("no active network — run `chakramcp networks use <name>` first"))?;
    let app_url = cfg
        .network(&network_name)
        .ok_or_else(|| anyhow!("active network '{network_name}' has no app_url"))?
        .app_url
        .trim_end_matches('/')
        .to_string();

    let http = reqwest::Client::builder()
        .user_agent(concat!("chakramcp-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()?;

    // ─── Step 1: ask for a code ───────────────────────────────────
    let auth_url = format!("{app_url}/oauth/device_authorization");
    let auth: DeviceAuthResponse = http
        .post(&auth_url)
        .json(&DeviceAuthRequest {
            persona: args.persona.as_deref(),
            agent_slug_hint: args.agent_slug.as_deref(),
            agent_display_name_hint: args.display_name.as_deref(),
            agent_description_hint: args.description.as_deref(),
        })
        .send()
        .await
        .with_context(|| format!("POST {auth_url}"))?
        .error_for_status()
        .with_context(|| "device_authorization endpoint rejected the request")?
        .json()
        .await
        .context("decoding device_authorization response")?;

    // ─── Step 2: tell the human what to do ────────────────────────
    eprintln!();
    eprintln!("    Pair this agent — three equivalent ways:");
    eprintln!();
    eprintln!("    1. Scan from your phone (QR rendered server-side, no install):");
    eprintln!("       {}", auth.verification_uri_qr);
    eprintln!();
    eprintln!("    2. Click on this device:");
    eprintln!("       {}", auth.verification_uri_complete);
    eprintln!();
    eprintln!("    3. Or type code at {}:", auth.verification_uri);
    eprintln!("       {}", auth.user_code);
    eprintln!();

    // Try to auto-open the QR URL — best UX is "QR on this screen,
    // user scans from phone, signs in once on the phone, approves."
    // The URL pages includes the underlying URL as text fallback.
    if !args.no_open {
        let _ = webbrowser::open(&auth.verification_uri_qr);
    }

    // ─── Step 3: poll until approved (or terminal) ────────────────
    let token_url = format!("{app_url}/oauth/token");
    let mut interval = Duration::from_secs(auth.interval.max(1) as u64);

    let pb = ui::spinner("waiting for approval…");
    let token = loop {
        sleep(interval).await;

        let resp = http
            .post(&token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &auth.device_code),
            ])
            .send()
            .await
            .with_context(|| format!("POST {token_url}"))?;

        let status = resp.status();
        if status.is_success() {
            let body: TokenResponse = resp
                .json()
                .await
                .context("decoding /oauth/token success response")?;
            break body;
        }

        // Per RFC 8628, errors during polling are application/json with
        // a flat `{ "error": "..." }` body. Decode and branch.
        let err: PollError = resp.json().await.context("decoding /oauth/token error")?;
        match err.error.as_str() {
            "authorization_pending" => {
                // keep polling at the current interval
            }
            "slow_down" => {
                // RFC 8628 §3.5: bump interval by 5s and continue.
                interval += Duration::from_secs(5);
            }
            "access_denied" => {
                pb.finish_and_clear();
                bail!("the user denied the pairing request");
            }
            "expired_token" => {
                pb.finish_and_clear();
                bail!("the pairing code expired before approval (rerun `chakramcp pair`)");
            }
            other => {
                pb.finish_and_clear();
                bail!("pairing failed: {other}");
            }
        }
    };
    pb.finish_and_clear();

    // ─── Step 4: persist the token ────────────────────────────────
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before UNIX epoch")?
        .as_secs() as i64;

    let net = cfg
        .network_mut(&network_name)
        .ok_or_else(|| anyhow!("network '{network_name}' vanished mid-flow"))?;
    net.auth.access_token = Some(token.access_token.clone());
    net.auth.access_token_expires_at = Some(now + token.expires_in);
    net.auth.api_key = None;
    cfg.save()?;

    let pretty = match (token.account_slug.as_deref(), token.agent_slug.as_deref()) {
        (Some(acct), Some(agent)) => format!("paired as {acct}/{agent}"),
        (Some(acct), None) => format!("paired under {acct}"),
        _ => "paired".to_string(),
    };
    ui::ok(&pretty);
    eprintln!(
        "    The token is saved in {} under network '{network_name}'.",
        crate::config::config_path()?.display()
    );
    eprintln!("    Run `chakramcp whoami` to verify.");

    Ok(())
}
