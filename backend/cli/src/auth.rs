//! `chakramcp login` — OAuth 2.1 + PKCE flow with a loopback redirect
//! per RFC 8252. The CLI:
//!   1. Reads /.well-known/oauth-authorization-server from the server
//!   2. Dynamically registers itself if it doesn't have a client_id yet
//!   3. Generates a PKCE pair and a random state
//!   4. Binds a TCP listener on a free port and opens the user's browser
//!      to the authorization endpoint with redirect_uri pointing here
//!   5. Captures the ?code=…&state=… on the loopback callback
//!   6. POSTs to /token with the verifier; saves the access token to config

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::CliConfig;

const CLIENT_NAME: &str = "ChakraMCP CLI";

#[derive(Debug, Deserialize)]
struct ServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
struct RegisterRequest {
    client_name: &'static str,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: &'static str,
    scope: &'static str,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

pub async fn login(cfg: &mut CliConfig) -> Result<String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("chakramcp-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()?;

    // 1. Discovery.
    let meta_url = format!("{}/.well-known/oauth-authorization-server", cfg.server.app_url.trim_end_matches('/'));
    let meta: ServerMetadata = http
        .get(&meta_url)
        .send()
        .await
        .with_context(|| format!("fetching {meta_url}"))?
        .error_for_status()?
        .json()
        .await?;

    // 2. Bind the loopback callback on a free port up front so we can use
    //    the resolved port in the redirect_uri at registration time.
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("binding loopback callback listener")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    // 3. Register the CLI as an OAuth client (or reuse a previous registration
    //    if the redirect_uri matches).
    let client_id = match (&cfg.server.oauth_client_id, meta.registration_endpoint.as_ref()) {
        (Some(id), _) => id.clone(),
        (None, Some(reg_endpoint)) => {
            let resp: RegisterResponse = http
                .post(reg_endpoint)
                .json(&RegisterRequest {
                    client_name: CLIENT_NAME,
                    redirect_uris: vec![redirect_uri.clone()],
                    token_endpoint_auth_method: "none",
                    scope: "relay.full",
                })
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            cfg.server.oauth_client_id = Some(resp.client_id.clone());
            resp.client_id
        }
        (None, None) => {
            bail!("server doesn't advertise registration_endpoint and we have no client_id stashed")
        }
    };

    // 4. PKCE pair + state.
    let mut verifier_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize());

    let mut state_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut state_bytes);
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(state_bytes);

    let auth_url = {
        let mut u = url::Url::parse(&meta.authorization_endpoint)?;
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair("client_id", &client_id);
            q.append_pair("redirect_uri", &redirect_uri);
            q.append_pair("code_challenge", &challenge);
            q.append_pair("code_challenge_method", "S256");
            q.append_pair("state", &state);
            q.append_pair("scope", "relay.full");
        }
        u.to_string()
    };

    eprintln!("Opening browser to sign you in…");
    eprintln!("  {auth_url}");
    if let Err(err) = webbrowser::open(&auth_url) {
        eprintln!(
            "Couldn't auto-open the browser ({err}). Open the URL above manually."
        );
    }

    // 5. Capture the callback. Single-shot — we accept exactly one connection
    //    and respond with a tiny success page.
    listener
        .set_nonblocking(false)
        .context("setting listener blocking mode")?;
    let (mut sock, _peer) = listener.accept().context("waiting for OAuth callback")?;
    sock.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut buf = [0u8; 4096];
    let n = sock.read(&mut buf).context("reading callback request")?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let request_line = req.lines().next().unwrap_or("");

    let path_and_query = request_line.split_whitespace().nth(1).unwrap_or("/");
    let parsed = url::Url::parse(&format!("http://localhost{path_and_query}"))?;
    let returned_state = parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    let code = parsed
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned());
    let oauth_error = parsed
        .query_pairs()
        .find(|(k, _)| k == "error")
        .map(|(_, v)| v.into_owned());

    let body = match (&code, &oauth_error) {
        (Some(_), _) => SUCCESS_HTML,
        _ => FAILED_HTML,
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = sock.write_all(response.as_bytes());
    let _ = sock.flush();
    drop(sock);

    if let Some(err) = oauth_error {
        bail!("OAuth flow returned error: {err}");
    }
    if returned_state != state {
        bail!("state mismatch on OAuth callback — possible CSRF; aborting");
    }
    let code = code.ok_or_else(|| anyhow!("OAuth callback missing 'code' parameter"))?;

    // 6. Exchange.
    let token: TokenResponse = http
        .post(&meta.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("redirect_uri", &redirect_uri),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    cfg.auth.access_token = Some(token.access_token.clone());
    cfg.auth.access_token_expires_at = Some(now + token.expires_in);
    // Clear any stale API key so Bearer order is deterministic.
    cfg.auth.api_key = None;
    cfg.save()?;

    Ok(token.access_token)
}

const SUCCESS_HTML: &str = r#"<!doctype html><meta charset=utf-8><title>Signed in</title>
<style>body{font-family:-apple-system,system-ui,sans-serif;display:grid;place-items:center;min-height:100dvh;margin:0;background:#f7f0e8;color:#2b2421}main{text-align:center;padding:2rem}h1{margin:0 0 .5rem;font-size:1.6rem}p{margin:0;color:#6b5f57}</style>
<main><h1>You're signed in.</h1><p>You can close this tab and return to the terminal.</p></main>"#;

const FAILED_HTML: &str = r#"<!doctype html><meta charset=utf-8><title>Sign-in failed</title>
<style>body{font-family:-apple-system,system-ui,sans-serif;display:grid;place-items:center;min-height:100dvh;margin:0;background:#f7f0e8;color:#2b2421}main{text-align:center;padding:2rem}h1{margin:0 0 .5rem;font-size:1.6rem}p{margin:0;color:#a13d3d}</style>
<main><h1>Sign-in failed.</h1><p>Check the terminal for details.</p></main>"#;
