use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::Value;

use crate::client::ApiClient;
use crate::config::CliConfig;

#[derive(Parser, Debug)]
pub struct Args {
    /// The API key (`ck_…`) to save.
    #[arg(long)]
    pub api_key: String,
}

pub async fn run(args: Args, cfg: &mut CliConfig) -> Result<()> {
    if !args.api_key.starts_with("ck_") {
        bail!("API key must start with `ck_`");
    }
    cfg.auth.api_key = Some(args.api_key);
    cfg.auth.access_token = None;
    cfg.auth.access_token_expires_at = None;
    cfg.save()?;

    // Verify by calling /v1/me.
    let api = ApiClient::new(cfg.clone())?;
    let me: Value = api
        .get_app("/v1/me")
        .await
        .context("API key didn't work — /v1/me rejected it")?;
    eprintln!(
        "Configured. Signed in as {}.",
        me.pointer("/user/email").and_then(|v| v.as_str()).unwrap_or("?")
    );
    Ok(())
}
