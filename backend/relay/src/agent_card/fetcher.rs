//! Push-mode upstream Agent Card fetcher.
//!
//! For push-mode agents (those with a public A2A endpoint declared via
//! `agents.agent_card_url`), we periodically GET the upstream card,
//! normalize it for republish under our domain, and cache the result.
//!
//! Normalization rules (per discovery spec §"Agent Card hosting model"
//! and §"Upstream auth-scheme handling on republish"):
//!
//! - `supported_interfaces` is REPLACED with a single entry pointing
//!   at our relay URL. Upstream's interfaces (which would let A2A
//!   clients bypass our policy proxy) are NOT propagated.
//! - `security_schemes` is REPLACED with a single `chakramcp_bearer`
//!   scheme. Upstream's `apiKey`/`oauth2`/`oidc` are not propagated;
//!   they have no effect because all calls flow through our relay.
//! - `signatures` from upstream are preserved (multiple signatures
//!   per card are spec-allowed). Our own signature is added at serve
//!   time by the published_cards handler, NOT here.
//! - `extra` (forward-compat unknown fields) is preserved verbatim.
//!
//! Caching:
//! - `If-None-Match` is sent on subsequent fetches when we have an
//!   etag. 304 returns `FetchOutcome::NotModified` so the caller can
//!   keep using the cached row without rewriting.
//! - Upstream `Cache-Control: max-age=<n>` is honored as a *minimum*
//!   refresh interval, but **HARD-CLAMPED** to 3600 (60 min) so
//!   misbehaving upstreams can't push our agent into a "stale"
//!   health state. See discovery spec D2d clamping rule.

use std::time::Duration;

use reqwest::header;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::synthesizer::SECURITY_SCHEME_NAME;
use super::types::{
    AgentCard, AgentInterface, HttpAuthDetails, HttpAuthSecurityScheme, SecurityRequirement,
    SecurityScheme, A2A_PROTOCOL_VERSION, PROTOCOL_BINDING_JSONRPC,
};

/// Hard upper bound on how often we re-fetch, regardless of what the
/// upstream Cache-Control says. Discovery spec §"Push card refresh"
/// → "clamped to ≤ 60 min so health thresholds remain meaningful."
pub const MAX_REFRESH_INTERVAL_SECONDS: u64 = 3600;

/// Default Fetcher network timeout. Upstream Agent Card endpoints
/// should be cheap; anything over a few seconds suggests degradation.
pub const FETCH_TIMEOUT_SECONDS: u64 = 8;

/// Outcome of a fetch attempt.
#[derive(Debug)]
pub enum FetchOutcome {
    /// Upstream returned a fresh card (200). Includes the normalized
    /// version ready for storage and the etag/max-age for future
    /// `If-None-Match`.
    Fresh {
        /// Card after URL/auth substitution — what we'll serve.
        normalized: AgentCard,
        /// Original upstream card — kept around for audit. We don't
        /// republish the upstream's url/auth, but we do preserve the
        /// upstream's signature on the wire.
        upstream: AgentCard,
        /// Upstream `ETag` header for the next fetch's `If-None-Match`.
        etag: Option<String>,
        /// Server-clamped max-age. Used by the refresh-job scheduler
        /// to compute next-fetch-due timestamp.
        max_age_seconds: u64,
    },
    /// Upstream returned 304. Caller should keep the existing cache.
    NotModified,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("upstream returned status {0}")]
    UpstreamStatus(u16),
    #[error("upstream returned a malformed Agent Card: {0}")]
    InvalidUpstreamCard(String),
    #[error("network/transport error: {0}")]
    Transport(String),
    #[error("upstream URL is malformed or scheme is not http(s): {0}")]
    BadUrl(String),
}

/// HTTP client that fetches upstream A2A Agent Cards.
///
/// Stateless apart from a tuned `reqwest::Client`. Construct once at
/// startup (in RelayState) and share. Cheap to clone — `reqwest::Client`
/// is `Arc<Inner>` internally.
#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "chakramcp-relay/",
                env!("CARGO_PKG_VERSION"),
                " (+https://chakramcp.com)"
            ))
            .timeout(Duration::from_secs(FETCH_TIMEOUT_SECONDS))
            // Don't follow redirects to a different host without
            // checking — for v1 we trust the agent_card_url the
            // operator registered. Defensive but minimal: cap at 3 hops.
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("reqwest client builds with default options");
        Self { client }
    }

    /// Fetch + normalize. `if_none_match` is the etag we have cached
    /// from a previous fetch (None on first fetch).
    pub async fn fetch(
        &self,
        upstream_url: &str,
        if_none_match: Option<&str>,
        relay_base_url: &str,
        account_slug: &str,
        agent_slug: &str,
    ) -> Result<FetchOutcome, FetchError> {
        if !(upstream_url.starts_with("http://") || upstream_url.starts_with("https://")) {
            return Err(FetchError::BadUrl(upstream_url.to_string()));
        }

        let mut req = self.client.get(upstream_url);
        if let Some(etag) = if_none_match {
            req = req.header(header::IF_NONE_MATCH, etag);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| FetchError::Transport(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::NotModified);
        }
        if !status.is_success() {
            return Err(FetchError::UpstreamStatus(status.as_u16()));
        }

        let etag = resp
            .headers()
            .get(header::ETAG)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let max_age_seconds = parse_cache_control_max_age(resp.headers().get(header::CACHE_CONTROL))
            .unwrap_or(MAX_REFRESH_INTERVAL_SECONDS)
            .min(MAX_REFRESH_INTERVAL_SECONDS);

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        let upstream: AgentCard = serde_json::from_slice(&bytes).map_err(|e| {
            FetchError::InvalidUpstreamCard(format!("json parse failed: {e}"))
        })?;

        let normalized =
            normalize_for_publish(upstream.clone(), relay_base_url, account_slug, agent_slug);

        Ok(FetchOutcome::Fresh {
            normalized,
            upstream,
            etag,
            max_age_seconds,
        })
    }
}

/// Extract `max-age=<seconds>` from a `Cache-Control` header value.
/// Returns None if the header is absent or doesn't contain max-age.
fn parse_cache_control_max_age(value: Option<&header::HeaderValue>) -> Option<u64> {
    let s = value.and_then(|v| v.to_str().ok())?;
    for part in s.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("max-age=").or_else(|| part.strip_prefix("s-maxage=")) {
            return v.trim().parse::<u64>().ok();
        }
    }
    None
}

/// Convert an upstream-fetched card into the form we publish under
/// our domain. Non-destructive: takes ownership and returns a
/// transformed value.
///
/// Rules:
/// - `supported_interfaces` REPLACED with one entry pointing at our
///   relay's JSON-RPC endpoint. Upstream's interface URLs are NOT
///   exposed publicly — they're internal state used only by D5's
///   forwarder when proxying authorized calls.
/// - `security_schemes` REPLACED with a single `chakramcp_bearer`
///   scheme. Upstream's are dropped on publish.
/// - `security_requirements` REPLACED to require chakramcp_bearer.
/// - `signatures` PRESERVED (upstream's signature objects survive).
///   Our own signature is appended later by the publish handler.
/// - All other fields (name, description, version, capabilities,
///   skills, default modes, provider, icon, extra) PASS THROUGH.
pub fn normalize_for_publish(
    mut card: AgentCard,
    relay_base_url: &str,
    account_slug: &str,
    agent_slug: &str,
) -> AgentCard {
    let base = relay_base_url.trim_end_matches('/');
    let url = format!("{base}/agents/{account_slug}/{agent_slug}/a2a/jsonrpc");

    card.supported_interfaces = vec![AgentInterface {
        url,
        protocol_binding: PROTOCOL_BINDING_JSONRPC.to_string(),
        tenant: None,
        protocol_version: A2A_PROTOCOL_VERSION.to_string(),
        extra: Default::default(),
    }];

    let mut schemes = std::collections::BTreeMap::new();
    schemes.insert(SECURITY_SCHEME_NAME.to_string(), bearer_jwt_scheme());
    card.security_schemes = schemes;

    let mut req = std::collections::BTreeMap::new();
    req.insert(SECURITY_SCHEME_NAME.to_string(), Vec::<String>::new());
    card.security_requirements = vec![SecurityRequirement { schemes: req }];

    card
}

fn bearer_jwt_scheme() -> SecurityScheme {
    SecurityScheme::Http(HttpAuthSecurityScheme {
        http: HttpAuthDetails {
            description: Some(
                "ChakraMCP-issued bearer token (API key or OAuth-issued JWT).".to_string(),
            ),
            scheme: "Bearer".to_string(),
            bearer_format: Some("JWT".to_string()),
            extra: Default::default(),
        },
    })
}

/// Persistent state captured per fetch. Stored in
/// `agents.agent_card_cached` as a single JSON blob; we don't shard
/// across additional columns because the published-card handler
/// always wants all of these together.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedCardEnvelope {
    /// What we'll publish (normalized, ready to be signed).
    pub normalized: AgentCard,
    /// What upstream returned (audit trail; not directly published).
    pub upstream: AgentCard,
    /// For next fetch's If-None-Match.
    pub etag: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("fetch failed: {0}")]
    Fetch(#[from] FetchError),
    #[error("agent has no agent_card_url (mode != 'push'?)")]
    NotPushMode,
    #[error("agent not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Fetch the upstream card for `agent_id` and cache the normalized
/// version into `agents.agent_card_cached`. Idempotent — caller is
/// expected to call this on first push-mode card request, and the
/// refresh job (D2e) runs it periodically thereafter.
///
/// Returns the normalized card so the caller can serve it without a
/// second DB read.
pub async fn cache_card_for_agent(
    pool: &PgPool,
    fetcher: &Fetcher,
    agent_id: Uuid,
    relay_base_url: &str,
) -> Result<AgentCard, CacheError> {
    let row = sqlx::query!(
        r#"
        SELECT a.agent_card_url, a.slug AS agent_slug,
               acc.slug AS account_slug,
               a.agent_card_cached
          FROM agents a
          JOIN accounts acc ON acc.id = a.account_id
         WHERE a.id = $1
           AND a.tombstoned_at IS NULL
           AND acc.tombstoned_at IS NULL
        "#,
        agent_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(CacheError::NotFound)?;

    let url = row.agent_card_url.ok_or(CacheError::NotPushMode)?;

    // Pull the previous etag, if any, from the cached envelope.
    let prior_etag: Option<String> = row.agent_card_cached.as_ref().and_then(|v| {
        serde_json::from_value::<CachedCardEnvelope>(v.clone())
            .ok()
            .and_then(|env| env.etag)
    });

    let outcome = fetcher
        .fetch(
            &url,
            prior_etag.as_deref(),
            relay_base_url,
            &row.account_slug,
            &row.agent_slug,
        )
        .await?;

    let envelope = match outcome {
        FetchOutcome::Fresh {
            normalized,
            upstream,
            etag,
            max_age_seconds: _max_age,
        } => {
            let upstream_signed = !upstream.signatures.is_empty();
            let envelope = CachedCardEnvelope {
                normalized: normalized.clone(),
                upstream,
                etag,
            };
            let envelope_json = serde_json::to_value(&envelope).map_err(|e| {
                CacheError::Db(sqlx::Error::Configuration(
                    format!("serialize envelope: {e}").into(),
                ))
            })?;

            sqlx::query!(
                r#"
                UPDATE agents
                   SET agent_card_cached = $2,
                       agent_card_fetched_at = now(),
                       agent_card_signed = $3,
                       -- We don't verify upstream signatures in v1.
                       -- Set false explicitly to make this clear in audit.
                       agent_card_signature_verified = false
                 WHERE id = $1
                "#,
                agent_id,
                envelope_json,
                upstream_signed,
            )
            .execute(pool)
            .await?;

            envelope
        }
        FetchOutcome::NotModified => {
            // Bump the timestamp so health checks see the fetch
            // attempt, but leave the cached body and signed flags
            // untouched.
            sqlx::query!(
                "UPDATE agents SET agent_card_fetched_at = now() WHERE id = $1",
                agent_id,
            )
            .execute(pool)
            .await?;
            row.agent_card_cached
                .ok_or_else(|| {
                    CacheError::Db(sqlx::Error::Configuration(
                        "304 received but no cached envelope to reuse".into(),
                    ))
                })
                .and_then(|v| {
                    serde_json::from_value::<CachedCardEnvelope>(v).map_err(|e| {
                        CacheError::Db(sqlx::Error::Configuration(
                            format!("parse cached envelope: {e}").into(),
                        ))
                    })
                })?
        }
    };

    Ok(envelope.normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::get;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Test upstream server. Counts requests so tests can assert
    /// idempotence / etag behavior, returns whatever JSON the test
    /// configures via the response body string.
    #[allow(dead_code)] // body/etag/cache_control fields are kept for inspection-by-debugger
    struct TestUpstream {
        addr: SocketAddr,
        body: Arc<String>,
        etag: Option<&'static str>,
        cache_control: Option<&'static str>,
        request_count: Arc<AtomicUsize>,
        last_if_none_match: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl TestUpstream {
        async fn start(
            body: String,
            etag: Option<&'static str>,
            cache_control: Option<&'static str>,
        ) -> Self {
            let body = Arc::new(body);
            let request_count = Arc::new(AtomicUsize::new(0));
            let last_if_none_match = Arc::new(std::sync::Mutex::new(None));

            #[derive(Clone)]
            struct AppState {
                body: Arc<String>,
                etag: Option<&'static str>,
                cache_control: Option<&'static str>,
                request_count: Arc<AtomicUsize>,
                last_if_none_match: Arc<std::sync::Mutex<Option<String>>>,
            }

            async fn handler(
                State(s): State<AppState>,
                headers: HeaderMap,
            ) -> axum::response::Response {
                use axum::http::{HeaderName, HeaderValue, StatusCode};
                use axum::response::IntoResponse;
                s.request_count.fetch_add(1, Ordering::SeqCst);
                let inm = headers
                    .get(axum::http::header::IF_NONE_MATCH)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                *s.last_if_none_match.lock().unwrap() = inm.clone();
                if let (Some(client_etag), Some(server_etag)) = (inm.as_deref(), s.etag) {
                    if client_etag == server_etag {
                        return (StatusCode::NOT_MODIFIED, [] as [(HeaderName, &str); 0])
                            .into_response();
                    }
                }
                let mut hdrs = HeaderMap::new();
                hdrs.insert(
                    axum::http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                if let Some(e) = s.etag {
                    hdrs.insert(
                        axum::http::header::ETAG,
                        HeaderValue::from_str(e).unwrap(),
                    );
                }
                if let Some(cc) = s.cache_control {
                    hdrs.insert(
                        axum::http::header::CACHE_CONTROL,
                        HeaderValue::from_str(cc).unwrap(),
                    );
                }
                (StatusCode::OK, hdrs, (*s.body).clone()).into_response()
            }

            let state = AppState {
                body: body.clone(),
                etag,
                cache_control,
                request_count: request_count.clone(),
                last_if_none_match: last_if_none_match.clone(),
            };
            let app = axum::Router::new()
                .route("/.well-known/agent-card.json", get(handler))
                .with_state(state);

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app).await.ok();
            });

            Self {
                addr,
                body,
                etag,
                cache_control,
                request_count,
                last_if_none_match,
            }
        }

        fn url(&self) -> String {
            format!("http://{}/.well-known/agent-card.json", self.addr)
        }

        fn request_count(&self) -> usize {
            self.request_count.load(Ordering::SeqCst)
        }
    }

    fn sample_upstream_card_json() -> String {
        serde_json::json!({
            "name": "Travel Planner",
            "description": "Plans trips.",
            "supported_interfaces": [
                {
                    "url": "https://travel.example.com/a2a/v1",
                    "protocol_binding": "JSONRPC",
                    "protocol_version": "0.3"
                },
                {
                    "url": "https://travel.example.com/a2a/v1.grpc",
                    "protocol_binding": "GRPC",
                    "protocol_version": "0.3"
                }
            ],
            "version": "2.1.0",
            "capabilities": { "streaming": true, "push_notifications": false },
            "security_schemes": {
                "upstream_oauth": {
                    "oauth2": { "flows": { "client_credentials": {} } }
                }
            },
            "security_requirements": [{ "upstream_oauth": ["read"] }],
            "default_input_modes": ["application/json"],
            "default_output_modes": ["application/json"],
            "skills": [
                {
                    "id": "plan-trip",
                    "name": "Plan Trip",
                    "description": "Plans an itinerary.",
                    "tags": ["travel"],
                    "examples": ["Plan a 5-day trip to Lisbon."]
                }
            ],
            "signatures": [
                { "protected": "upstream-protected", "signature": "upstream-signature" }
            ]
        })
        .to_string()
    }

    #[tokio::test]
    async fn fetch_and_normalize_substitutes_url_and_auth() {
        let server = TestUpstream::start(
            sample_upstream_card_json(),
            Some("\"v1-etag\""),
            Some("public, max-age=120"),
        )
        .await;

        let f = Fetcher::new();
        let outcome = f
            .fetch(
                &server.url(),
                None,
                "https://chakramcp.com",
                "acme-corp",
                "travel-planner",
            )
            .await
            .unwrap();

        match outcome {
            FetchOutcome::Fresh {
                normalized,
                upstream,
                etag,
                max_age_seconds,
            } => {
                // URL substitution: ours, not upstream's.
                assert_eq!(normalized.supported_interfaces.len(), 1);
                assert_eq!(
                    normalized.supported_interfaces[0].url,
                    "https://chakramcp.com/agents/acme-corp/travel-planner/a2a/jsonrpc"
                );
                assert_eq!(
                    normalized.supported_interfaces[0].protocol_binding,
                    "JSONRPC"
                );

                // Security: ours, not upstream's.
                assert_eq!(normalized.security_schemes.len(), 1);
                assert!(normalized.security_schemes.contains_key("chakramcp_bearer"));
                assert!(!normalized.security_schemes.contains_key("upstream_oauth"));

                // Pass-through fields preserved.
                assert_eq!(normalized.name, "Travel Planner");
                assert_eq!(normalized.version, "2.1.0");
                assert_eq!(normalized.skills.len(), 1);
                assert_eq!(normalized.skills[0].id, "plan-trip");

                // Upstream signature preserved on the normalized card
                // (we'll add ours alongside at serve time).
                assert_eq!(normalized.signatures.len(), 1);
                assert_eq!(normalized.signatures[0].signature, "upstream-signature");

                // Upstream payload kept for audit.
                assert_eq!(upstream.supported_interfaces.len(), 2);
                assert_eq!(
                    upstream.supported_interfaces[0].url,
                    "https://travel.example.com/a2a/v1"
                );

                assert_eq!(etag.as_deref(), Some("\"v1-etag\""));
                assert_eq!(max_age_seconds, 120);
            }
            other => panic!("expected Fresh, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cache_control_max_age_clamped_at_one_hour() {
        let server = TestUpstream::start(
            sample_upstream_card_json(),
            None,
            Some("public, max-age=86400"), // 24h — must be clamped to 3600
        )
        .await;
        let f = Fetcher::new();
        let outcome = f
            .fetch(&server.url(), None, "https://r", "a", "b")
            .await
            .unwrap();
        match outcome {
            FetchOutcome::Fresh {
                max_age_seconds, ..
            } => assert_eq!(max_age_seconds, MAX_REFRESH_INTERVAL_SECONDS),
            other => panic!("expected Fresh, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_cache_control_defaults_to_clamp() {
        let server = TestUpstream::start(sample_upstream_card_json(), None, None).await;
        let f = Fetcher::new();
        let outcome = f
            .fetch(&server.url(), None, "https://r", "a", "b")
            .await
            .unwrap();
        match outcome {
            FetchOutcome::Fresh { max_age_seconds, .. } => {
                assert_eq!(max_age_seconds, MAX_REFRESH_INTERVAL_SECONDS);
            }
            other => panic!("expected Fresh, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn etag_round_trip_returns_not_modified() {
        let server =
            TestUpstream::start(sample_upstream_card_json(), Some("\"v1-etag\""), None).await;
        let f = Fetcher::new();
        // First fetch — Fresh.
        let _ = f
            .fetch(&server.url(), None, "https://r", "a", "b")
            .await
            .unwrap();
        // Second fetch — with the etag the upstream gave us.
        let outcome = f
            .fetch(&server.url(), Some("\"v1-etag\""), "https://r", "a", "b")
            .await
            .unwrap();
        assert!(matches!(outcome, FetchOutcome::NotModified));
        assert_eq!(server.request_count(), 2);
        assert_eq!(
            server.last_if_none_match.lock().unwrap().as_deref(),
            Some("\"v1-etag\"")
        );
    }

    #[tokio::test]
    async fn malformed_upstream_json_is_invalid_card_error() {
        let server = TestUpstream::start("{not json".to_string(), None, None).await;
        let f = Fetcher::new();
        let r = f
            .fetch(&server.url(), None, "https://r", "a", "b")
            .await;
        assert!(matches!(r, Err(FetchError::InvalidUpstreamCard(_))));
    }

    #[tokio::test]
    async fn upstream_5xx_is_status_error() {
        // Build a server that always 500s.
        async fn always_500() -> axum::http::StatusCode {
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        }
        let app = axum::Router::new().route("/.well-known/agent-card.json", get(always_500));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let f = Fetcher::new();
        let r = f
            .fetch(
                &format!("http://{addr}/.well-known/agent-card.json"),
                None,
                "https://r",
                "a",
                "b",
            )
            .await;
        assert!(matches!(r, Err(FetchError::UpstreamStatus(500))));
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let f = Fetcher::new();
        let r = f
            .fetch("ftp://upstream.example.com/card", None, "https://r", "a", "b")
            .await;
        assert!(matches!(r, Err(FetchError::BadUrl(_))));
    }

    #[test]
    fn normalize_replaces_supported_interfaces_with_relay_url() {
        let raw: AgentCard = serde_json::from_str(&sample_upstream_card_json()).unwrap();
        let n = normalize_for_publish(raw, "https://chakramcp.com", "acme", "alice");
        assert_eq!(n.supported_interfaces.len(), 1);
        assert_eq!(
            n.supported_interfaces[0].url,
            "https://chakramcp.com/agents/acme/alice/a2a/jsonrpc"
        );
    }

    #[test]
    fn normalize_replaces_security_with_chakramcp_bearer() {
        let raw: AgentCard = serde_json::from_str(&sample_upstream_card_json()).unwrap();
        let n = normalize_for_publish(raw, "https://r", "a", "b");
        assert_eq!(n.security_schemes.len(), 1);
        assert!(n.security_schemes.contains_key("chakramcp_bearer"));
        // Old schemes gone.
        assert!(!n.security_schemes.contains_key("upstream_oauth"));
        assert_eq!(n.security_requirements.len(), 1);
        assert!(n.security_requirements[0]
            .schemes
            .contains_key("chakramcp_bearer"));
    }

    #[test]
    fn normalize_preserves_upstream_signatures() {
        let raw: AgentCard = serde_json::from_str(&sample_upstream_card_json()).unwrap();
        let n = normalize_for_publish(raw, "https://r", "a", "b");
        assert_eq!(n.signatures.len(), 1);
        assert_eq!(n.signatures[0].signature, "upstream-signature");
    }

    #[test]
    fn normalize_preserves_pass_through_fields() {
        let raw: AgentCard = serde_json::from_str(&sample_upstream_card_json()).unwrap();
        let n = normalize_for_publish(raw, "https://r", "a", "b");
        assert_eq!(n.name, "Travel Planner");
        assert_eq!(n.description, "Plans trips.");
        assert_eq!(n.version, "2.1.0");
        assert_eq!(n.capabilities.streaming, Some(true));
        assert_eq!(n.skills.len(), 1);
        assert_eq!(n.default_input_modes, vec!["application/json"]);
    }

    #[test]
    fn parse_cache_control_max_age_extracts_value() {
        let v = header::HeaderValue::from_static("public, max-age=300, s-maxage=3600");
        assert_eq!(parse_cache_control_max_age(Some(&v)), Some(300));
        let v = header::HeaderValue::from_static("no-cache");
        assert_eq!(parse_cache_control_max_age(Some(&v)), None);
    }

    // ─── Persistence-side tests via #[sqlx::test] ─────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn cache_card_for_agent_stores_envelope_and_marks_signed(pool: PgPool) {
        let server =
            TestUpstream::start(sample_upstream_card_json(), Some("\"v1\""), None).await;

        let acct_id = Uuid::now_v7();
        let agent_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, 'acme-corp', 'Acme', 'individual')"#,
            acct_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, mode, agent_card_url)
               VALUES ($1, $2, 'travel-planner', 'Travel', 'push', $3)"#,
            agent_id,
            acct_id,
            server.url(),
        )
        .execute(&pool)
        .await
        .unwrap();

        let f = Fetcher::new();
        let normalized =
            cache_card_for_agent(&pool, &f, agent_id, "https://chakramcp.com")
                .await
                .unwrap();

        assert_eq!(
            normalized.supported_interfaces[0].url,
            "https://chakramcp.com/agents/acme-corp/travel-planner/a2a/jsonrpc"
        );

        let row = sqlx::query!(
            r#"SELECT agent_card_cached, agent_card_signed, agent_card_signature_verified,
                      agent_card_fetched_at
                 FROM agents WHERE id = $1"#,
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(row.agent_card_cached.is_some());
        assert!(row.agent_card_signed); // upstream had a signature
        assert!(!row.agent_card_signature_verified); // we don't verify upstream sigs in v1
        assert!(row.agent_card_fetched_at.is_some());

        // Envelope shape round-trips.
        let envelope: CachedCardEnvelope =
            serde_json::from_value(row.agent_card_cached.unwrap()).unwrap();
        assert_eq!(envelope.etag.as_deref(), Some("\"v1\""));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn cache_card_for_agent_uses_etag_on_subsequent_fetches(pool: PgPool) {
        let server =
            TestUpstream::start(sample_upstream_card_json(), Some("\"v1\""), None).await;

        let acct_id = Uuid::now_v7();
        let agent_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, 'acme-corp', 'Acme', 'individual')"#,
            acct_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, mode, agent_card_url)
               VALUES ($1, $2, 'travel-planner', 'Travel', 'push', $3)"#,
            agent_id,
            acct_id,
            server.url(),
        )
        .execute(&pool)
        .await
        .unwrap();

        let f = Fetcher::new();
        let _ = cache_card_for_agent(&pool, &f, agent_id, "https://chakramcp.com")
            .await
            .unwrap();
        // Second fetch should send If-None-Match and get a 304;
        // function should still return the cached normalized card.
        let again = cache_card_for_agent(&pool, &f, agent_id, "https://chakramcp.com")
            .await
            .unwrap();
        assert_eq!(
            again.supported_interfaces[0].url,
            "https://chakramcp.com/agents/acme-corp/travel-planner/a2a/jsonrpc"
        );
        assert_eq!(server.request_count(), 2);
        assert_eq!(
            server.last_if_none_match.lock().unwrap().as_deref(),
            Some("\"v1\"")
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn cache_card_for_agent_rejects_non_push(pool: PgPool) {
        let acct_id = Uuid::now_v7();
        let agent_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, 'acme', 'Acme', 'individual')"#,
            acct_id,
        )
        .execute(&pool)
        .await
        .unwrap();
        // Default mode is 'pull' with no agent_card_url.
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name)
               VALUES ($1, $2, 'pull-agent', 'Pull')"#,
            agent_id,
            acct_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let f = Fetcher::new();
        let r = cache_card_for_agent(&pool, &f, agent_id, "https://r").await;
        assert!(matches!(r, Err(CacheError::NotPushMode)));
    }
}
