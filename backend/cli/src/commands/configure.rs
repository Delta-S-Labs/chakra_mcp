//! `chakramcp configure --api-key …` (or `--token …`) — headless / CI sign-in.
//!
//! `--token` takes a JWT the relay issued another way (device pairing, OAuth),
//! so nobody has to write it into `config.toml` by hand.
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
    #[arg(long, required_unless_present = "token")]
    pub api_key: Option<String>,

    /// A device or OAuth access token (a JWT) to save instead of an API key.
    #[arg(long, env = "CHAKRAMCP_TOKEN", conflicts_with = "api_key")]
    pub token: Option<String>,

    /// Network to attach this key to. Defaults to the active network,
    /// or creates a "public" network on first run.
    #[arg(long)]
    pub network: Option<String>,
}

/// The credential to store: an API key, or a token and when it expires.
enum Credential {
    ApiKey(String),
    Token { token: String, expires_at: i64 },
}

impl Credential {
    fn apply(&self, auth: &mut AuthConfig) {
        match self {
            Credential::ApiKey(key) => {
                auth.api_key = Some(key.clone());
                auth.access_token = None;
                auth.access_token_expires_at = None;
            }
            Credential::Token { token, expires_at } => {
                auth.access_token = Some(token.clone());
                auth.access_token_expires_at = Some(*expires_at);
                auth.api_key = None;
            }
        }
    }
}

/// A JWT's `exp`, read without verifying it (the relay verifies). A token
/// without one is kept until it's replaced.
fn token_expiry(token: &str) -> Result<i64> {
    use base64::Engine;
    let payload = token
        .split('.')
        .nth(1)
        .context("--token must be a JWT (header.payload.signature)")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .context("--token's payload isn't base64url")?;
    let claims: Value = serde_json::from_slice(&bytes).context("--token's payload isn't JSON")?;
    Ok(claims
        .get("exp")
        .and_then(Value::as_i64)
        .unwrap_or(i64::MAX))
}

pub async fn run(args: Args, cfg: &mut CliConfig) -> Result<()> {
    let credential = match (args.api_key, args.token) {
        (Some(key), _) => {
            if !key.starts_with("ck_") {
                bail!("API key must start with `ck_`");
            }
            Credential::ApiKey(key)
        }
        (None, Some(token)) => {
            let expires_at = token_expiry(&token)?;
            Credential::Token { token, expires_at }
        }
        (None, None) => bail!("pass --api-key or --token"),
    };

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
            _ => bail!("no network named '{target}' — add it first with `chakramcp networks add`"),
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
    credential.apply(&mut probe.network_mut(&target).unwrap().auth);
    probe.active = Some(target.clone());
    let api = ApiClient::new(probe)?;
    let me: Value = api
        .get_app("/v1/me")
        .await
        .context("the credential didn't work — /v1/me rejected it")?;

    credential.apply(&mut cfg.network_mut(&target).unwrap().auth);
    cfg.active = Some(target.clone());
    cfg.save()?;

    ui::ok(&format!(
        "configured. signed in as {} on '{target}'",
        me.pointer("/user/email")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    ));
    let _ = DEFAULT_NETWORK;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn jwt(claims: Value) -> String {
        let enc = |v: &Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).unwrap())
        };
        format!(
            "{}.{}.sig",
            enc(&serde_json::json!({"alg": "HS256"})),
            enc(&claims)
        )
    }

    #[test]
    fn a_tokens_expiry_comes_from_its_exp() {
        let t = jwt(serde_json::json!({ "sub": "u", "exp": 1_900_000_000 }));
        assert_eq!(token_expiry(&t).unwrap(), 1_900_000_000);
        let forever = jwt(serde_json::json!({ "sub": "u" }));
        assert_eq!(token_expiry(&forever).unwrap(), i64::MAX);
        assert!(token_expiry("ck_not_a_jwt").is_err());
    }

    #[test]
    fn api_key_or_token_but_not_both() {
        assert!(Args::try_parse_from(["configure", "--api-key", "ck_x"]).is_ok());
        assert!(Args::try_parse_from(["configure", "--token", "a.b.c"]).is_ok());
        assert!(
            Args::try_parse_from(["configure", "--api-key", "ck_x", "--token", "a.b.c"]).is_err()
        );
    }
}
