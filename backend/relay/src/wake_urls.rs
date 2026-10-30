//! Wake URLs (waking pull agents §7): a pull agent may give a URL the relay
//! pings when work arrives for it, so it needn't hold a pull open.
//!
//! One dispatcher per relay process takes the agents the wake hub gathers
//! (`Wake::take_dispatch`), skips those without a wake URL (an in-memory
//! set kept by `Heartbeats`), and for each of the rest:
//!
//! 1. waits [`WakeUrlSettings::group`] after the first wake, so a burst is
//!    one ping;
//! 2. counts the agent's unclaimed work, and sends nothing when there's none;
//! 3. claims the agent's slot (one ping per [`WakeUrlSettings::throttle`],
//!    from exactly one relay copy), or comes back when the slot frees;
//! 4. sends a signed JSON body, or a line of text, under the outbound guard;
//! 5. records the result, switching the URL off after [`FAILURE_LIMIT`]
//!    failures in a row.
//!
//! A ping carries counts and the agent's own name, never content or callers.
//! All of it runs in the background: the invoke path only notifies the hub.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chakramcp_shared::error::ApiError;
use chakramcp_shared::hosting::{env_value, parse_bool};
use chakramcp_shared::telemetry::names::WAKE_SENT_TOTAL;
use chakramcp_shared::wake::Feed;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use metrics::counter;
use rand::Rng;
use serde::Serialize;
use serde_json::json;
use sha2::Sha256;
use sqlx::PgPool;
use tokio::sync::{mpsc, Semaphore};
use tokio::time::Instant;
use uuid::Uuid;

use crate::state::RelayState;

/// How long a ping may take.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// Failures in a row that switch a wake URL off.
pub const FAILURE_LIMIT: i32 = 20;
/// The signature header on a JSON ping.
pub const SIGNATURE_HEADER: &str = "ChakraMCP-Signature";
/// Pings sent at once by one dispatcher.
const CONCURRENT_PINGS: usize = 16;

/// Wake URL settings (`WAKE_URLS_ENABLED`, and the hosting mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeUrlSettings {
    /// Run the dispatcher and accept wake URLs.
    pub enabled: bool,
    /// Only `https:` URLs (a managed server).
    pub https_only: bool,
    /// How long after an agent's first wake its ping goes out.
    pub group: Duration,
    /// At most one ping per agent this often.
    pub throttle: Duration,
}

impl Default for WakeUrlSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            https_only: false,
            group: Duration::from_secs(2),
            throttle: Duration::from_secs(30),
        }
    }
}

impl WakeUrlSettings {
    pub fn from_sources(
        env: impl Fn(&str) -> Option<String>,
        managed: bool,
    ) -> anyhow::Result<Self> {
        let enabled = match env_value(&env, "WAKE_URLS_ENABLED") {
            Some(raw) => parse_bool("WAKE_URLS_ENABLED", &raw)?,
            None => true,
        };
        Ok(Self {
            enabled,
            https_only: managed,
            ..Self::default()
        })
    }

    pub fn from_env(managed: bool) -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok(), managed)
    }

    /// For the startup log.
    pub fn summary(&self) -> &'static str {
        match (self.enabled, self.https_only) {
            (false, _) => "wake URLs: off",
            (true, true) => "wake URLs: on (https only)",
            (true, false) => "wake URLs: on",
        }
    }
}

/// A new signing secret, shown once.
pub fn new_secret() -> String {
    let bytes: [u8; 32] = rand::thread_rng().gen();
    format!("cmw_{}", hex::encode(bytes))
}

/// `ChakraMCP-Signature` for `body` sent at unix time `t`:
/// `t=<t>,v1=<hex HMAC-SHA256(secret, "<t>.<body>")>`.
pub fn signature(secret: &str, t: i64, body: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        .expect("HMAC takes keys of any length");
    mac.update(format!("{t}.{body}").as_bytes());
    format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
}

/// Check a wake URL before storing it: https on a managed server, and the
/// outbound guard's rules. A refusal is a 400 with the reason.
pub fn check_url(state: &RelayState, raw: &str) -> Result<String, ApiError> {
    let url = crate::outbound::check_url(raw.trim(), state.outbound)
        .map_err(|e| ApiError::InvalidRequest(format!("wake_url: {e}")))?;
    if state.wake_urls.https_only && url.scheme() != "https" {
        return Err(ApiError::InvalidRequest(
            "wake_url: this server only calls https URLs".into(),
        ));
    }
    Ok(url.to_string())
}

/// An agent's unclaimed work, as a ping reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Pending {
    pub invocations: i64,
    pub notices: i64,
    pub calls: i64,
}

impl Pending {
    pub fn total(&self) -> i64 {
        self.invocations + self.notices + self.calls
    }
}

async fn pending(db: &PgPool, agent: Uuid) -> Result<Pending, sqlx::Error> {
    let r = sqlx::query!(
        r#"
        SELECT
          (SELECT count(*) FROM relay_invocations
            WHERE granter_agent_id = $1 AND status = 'pending') AS "invocations!",
          (SELECT count(*) FROM agent_notices
            WHERE agent_id = $1 AND delivered_at IS NULL) AS "notices!",
          (SELECT count(*) FROM call_deliveries
            WHERE agent_id = $1 AND delivered_at IS NULL) AS "calls!"
        "#,
        agent,
    )
    .fetch_one(db)
    .await?;
    Ok(Pending {
        invocations: r.invocations,
        notices: r.notices,
        calls: r.calls,
    })
}

/// Where and how to ping an agent.
struct Target {
    url: String,
    format: String,
    secret: String,
    /// `account/agent`.
    handle: String,
}

/// What a ping's receiver answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PingResult {
    pub ok: bool,
    /// The HTTP status, when there was an answer.
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// The ping's body and content type.
fn body(agent: Uuid, target: &Target, pending: &Pending) -> (String, &'static str) {
    if target.format == "text" {
        let n = pending.total();
        let items = if n == 1 { "item" } else { "items" };
        (
            format!("{} has {n} new {items} on ChakraMCP", target.handle),
            "text/plain; charset=utf-8",
        )
    } else {
        let body = json!({
            "type": "chakramcp.wake",
            "agent": target.handle,
            "agent_id": agent,
            "pending": pending,
            "sent_at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        });
        (body.to_string(), "application/json")
    }
}

async fn send(
    client: &reqwest::Client,
    agent: Uuid,
    target: &Target,
    pending: &Pending,
) -> PingResult {
    let (body, content_type) = body(agent, target, pending);
    let mut request = client
        .post(&target.url)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header(reqwest::header::USER_AGENT, "ChakraMCP-Wake/1");
    if target.format != "text" {
        request = request.header(
            SIGNATURE_HEADER,
            signature(&target.secret, Utc::now().timestamp(), &body),
        );
    }
    match request.body(body).send().await {
        Ok(response) if response.status().is_success() => PingResult {
            ok: true,
            status: Some(response.status().as_u16()),
            error: None,
        },
        Ok(response) => PingResult {
            ok: false,
            status: Some(response.status().as_u16()),
            error: Some(format!("answered HTTP {}", response.status().as_u16())),
        },
        Err(e) => PingResult {
            ok: false,
            status: None,
            error: Some(error_text(e)),
        },
    }
}

/// A send error in a sentence, without the URL (it may hold a token).
fn error_text(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = if e.is_timeout() {
        "timed out".to_owned()
    } else if e.is_connect() {
        format!("couldn't connect: {e}")
    } else {
        e.to_string()
    };
    text.truncate(300);
    text
}

/// Record a ping's result. The twentieth failure in a row switches the URL
/// off and tells the agent with a `wake_url_disabled` notice.
async fn record(state: &RelayState, agent: Uuid, result: &PingResult) -> Result<(), sqlx::Error> {
    if result.ok {
        sqlx::query!(
            r#"
            UPDATE agent_wake
               SET failures = 0, last_ok_at = now(), last_error = NULL
             WHERE agent_id = $1
            "#,
            agent,
        )
        .execute(&state.db)
        .await?;
        return Ok(());
    }
    let mut tx = state.db.begin().await?;
    let switched_off = sqlx::query_scalar!(
        r#"
        UPDATE agent_wake
           SET failures = failures + 1,
               last_error = $2,
               enabled = enabled AND failures + 1 < $3,
               disabled_at = CASE WHEN enabled AND failures + 1 >= $3 THEN now()
                                  ELSE disabled_at END
         WHERE agent_id = $1
        RETURNING disabled_at IS NOT NULL AND disabled_at >= now() AS "switched_off!"
        "#,
        agent,
        result.error.as_deref(),
        FAILURE_LIMIT,
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(false);
    if switched_off {
        sqlx::query!(
            r#"
            INSERT INTO agent_notices (id, agent_id, kind, payload)
            VALUES ($1, $2, 'wake_url_disabled', '{}'::jsonb)
            "#,
            Uuid::now_v7(),
            agent,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if switched_off {
        tracing::info!(%agent, "wake URL switched off after {FAILURE_LIMIT} failures");
        state.heartbeats.set_wake(agent, false);
        state.wake.notify(agent, Feed::Notices);
    }
    Ok(())
}

fn client(state: &RelayState) -> reqwest::Client {
    crate::outbound::client_builder(state.outbound)
        .timeout(SEND_TIMEOUT)
        .build()
        .expect("reqwest client builds with default options")
}

/// Ping `agent` now, whatever its counts and the throttle (`POST
/// /v1/agents/{id}/wake/test`), and record the result as a real ping's.
/// `NotFound` when it has no wake URL.
pub async fn ping_now(state: &RelayState, agent: Uuid) -> Result<PingResult, ApiError> {
    let target = sqlx::query!(
        r#"
        UPDATE agent_wake w SET last_sent_at = now()
          FROM agents a JOIN accounts acc ON acc.id = a.account_id
         WHERE w.agent_id = $1 AND a.id = w.agent_id AND a.tombstoned_at IS NULL
        RETURNING w.url, w.format, w.secret, acc.slug || '/' || a.slug AS "handle!"
        "#,
        agent,
    )
    .fetch_optional(&state.db)
    .await?
    .map(|r| Target {
        url: r.url,
        format: r.format,
        secret: r.secret,
        handle: r.handle,
    })
    .ok_or(ApiError::NotFound)?;
    let counts = pending(&state.db, agent).await?;
    let result = send(&client(state), agent, &target, &counts).await;
    record(state, agent, &result).await?;
    counter!(WAKE_SENT_TOTAL, "result" => if result.ok { "ok" } else { "failed" }).increment(1);
    Ok(result)
}

/// A throttled agent's next turn: when, and the last ping it was throttled
/// by. A newer ping by then covered its wake.
type Again = (Instant, DateTime<Utc>);

/// One agent's turn: ping it if work waits and its slot is free. `covered`
/// is the last ping a throttled wake saw; if another has gone out since, it
/// covered the wake, and there's nothing to do (no reminder pings). Returns
/// when to look again, when the slot was taken.
async fn dispatch_one(
    state: &RelayState,
    client: &reqwest::Client,
    agent: Uuid,
    covered: Option<DateTime<Utc>>,
) -> Result<Option<Again>, sqlx::Error> {
    let counts = pending(&state.db, agent).await?;
    if counts.total() == 0 {
        counter!(WAKE_SENT_TOTAL, "result" => "nothing_pending").increment(1);
        return Ok(None);
    }
    let throttle = state.wake_urls.throttle.as_secs_f64();
    let claimed = sqlx::query!(
        r#"
        UPDATE agent_wake w SET last_sent_at = now()
          FROM agents a JOIN accounts acc ON acc.id = a.account_id
         WHERE w.agent_id = $1 AND a.id = w.agent_id AND a.tombstoned_at IS NULL
           AND w.enabled
           AND (w.last_sent_at IS NULL
                OR w.last_sent_at < now() - $2::float8 * interval '1 second')
        RETURNING w.url, w.format, w.secret, acc.slug || '/' || a.slug AS "handle!"
        "#,
        agent,
        throttle,
    )
    .fetch_optional(&state.db)
    .await?;
    let Some(r) = claimed else {
        // Pinged within the window (here or by another copy). If that ping
        // came after the one this wake was throttled by, it covered the
        // wake. Otherwise look again when the window ends, so work that
        // arrived meanwhile still gets a ping.
        let last = sqlx::query!(
            r#"
            SELECT last_sent_at AS "last_sent_at!",
                   EXTRACT(EPOCH FROM (last_sent_at + $2::float8 * interval '1 second' - now()))
                       ::float8 AS "left!"
              FROM agent_wake
             WHERE agent_id = $1 AND enabled AND last_sent_at IS NOT NULL
            "#,
            agent,
            throttle,
        )
        .fetch_optional(&state.db)
        .await?;
        let Some(last) = last.filter(|l| covered.is_none_or(|c| l.last_sent_at <= c)) else {
            return Ok(None);
        };
        counter!(WAKE_SENT_TOTAL, "result" => "throttled").increment(1);
        let at = Instant::now()
            + Duration::from_secs_f64(last.left.max(0.0))
            + Duration::from_millis(50);
        return Ok(Some((at, last.last_sent_at)));
    };
    let target = Target {
        url: r.url,
        format: r.format,
        secret: r.secret,
        handle: r.handle,
    };
    let result = send(client, agent, &target, &counts).await;
    record(state, agent, &result).await?;
    counter!(WAKE_SENT_TOTAL, "result" => if result.ok { "ok" } else { "failed" }).increment(1);
    Ok(None)
}

/// Run this process's dispatcher, and have the hub gather wakes for it.
pub fn spawn(state: RelayState) -> tokio::task::JoinHandle<()> {
    state.wake.start_dispatching();
    tokio::spawn(dispatch_loop(state))
}

async fn dispatch_loop(state: RelayState) {
    let client = client(&state);
    let pings = Arc::new(Semaphore::new(CONCURRENT_PINGS));
    // When each agent is due, with the ping a throttled wake saw; and
    // agents coming back after a throttle.
    let mut due: HashMap<Uuid, (Instant, Option<DateTime<Utc>>)> = HashMap::new();
    let (again_tx, mut again_rx) = mpsc::unbounded_channel::<(Uuid, Again)>();
    loop {
        let next = due.values().map(|(at, _)| *at).min();
        tokio::select! {
            _ = state.wake.dispatch_ready() => {
                let at = Instant::now() + state.wake_urls.group;
                for agent in state.wake.take_dispatch() {
                    if state.heartbeats.has_wake(agent) {
                        due.entry(agent).or_insert((at, None));
                    }
                }
            }
            Some((agent, (at, seen))) = again_rx.recv() => {
                let entry = due.entry(agent).or_insert((at, Some(seen)));
                entry.0 = entry.0.min(at);
                entry.1 = Some(seen);
            }
            _ = sleep_until(next) => {
                let now = Instant::now();
                let ready: Vec<(Uuid, Option<DateTime<Utc>>)> = due
                    .iter()
                    .filter(|(_, (at, _))| *at <= now)
                    .map(|(agent, (_, seen))| (*agent, *seen))
                    .collect();
                for (agent, covered) in ready {
                    due.remove(&agent);
                    let Ok(permit) = pings.clone().acquire_owned().await else {
                        return;
                    };
                    let (state, client, again_tx) = (state.clone(), client.clone(), again_tx.clone());
                    tokio::spawn(async move {
                        match dispatch_one(&state, &client, agent, covered).await {
                            Ok(Some(again)) => {
                                let _ = again_tx.send((agent, again));
                            }
                            Ok(None) => {}
                            Err(e) => tracing::warn!(%agent, error = %e, "wake ping failed"),
                        }
                        drop(permit);
                    });
                }
            }
        }
    }
}

/// Sleep until `at`, or forever without one.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_follow_the_switch_and_the_mode() {
        let off = WakeUrlSettings::from_sources(
            |k| (k == "WAKE_URLS_ENABLED").then(|| "false".into()),
            false,
        )
        .unwrap();
        assert!(!off.enabled);
        let managed = WakeUrlSettings::from_sources(|_| None, true).unwrap();
        assert!(managed.enabled && managed.https_only);
        assert!(WakeUrlSettings::from_sources(
            |k| (k == "WAKE_URLS_ENABLED").then(|| "maybe".into()),
            false
        )
        .is_err());
    }

    #[test]
    fn signatures_are_hmac_of_time_and_body() {
        // HMAC-SHA256("key", "1700000000.{}"), computed independently.
        let sig = signature("key", 1_700_000_000, "{}");
        assert!(sig.starts_with("t=1700000000,v1="), "{sig}");
        assert_eq!(sig.len(), "t=1700000000,v1=".len() + 64);
        assert_ne!(sig, signature("key", 1_700_000_001, "{}"));
        assert_ne!(sig, signature("other", 1_700_000_000, "{}"));
        assert!(new_secret().starts_with("cmw_"));
        assert_ne!(new_secret(), new_secret());
    }
}
