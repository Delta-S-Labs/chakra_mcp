//! `GET /agents/<account-slug>/<agent-slug>/.well-known/agent-card.json`
//! and the agent home pages that delegate to it.
//!
//! This is the A2A discovery surface. A network-visible agent's card is
//! public: anyone — friend, stranger, search engine, generic A2A client,
//! LLM autopiloting — can fetch it, and it lists the agent's
//! network-visible capabilities. A private or org-visible agent's card is
//! served only to a signed-in caller who may see that agent (a member of
//! its account, or of an org it's shared with), and lists the
//! capabilities that caller may see; to anyone else it's a 404, the same
//! as a missing agent. Method calls need auth too (`/a2a/jsonrpc`).
//!
//! Lookup flow (pull-mode, the only mode in D2c — push lands in D2d):
//!
//! 1. Resolve account slug → account row (404 if not found / tombstoned).
//! 2. Resolve agent slug under that account → agent row (404 if not
//!    found / tombstoned, 410 in a future revision when we want to
//!    distinguish "never existed" from "deleted").
//! 3. If `mode = 'push'` and we don't have a cached card yet, return
//!    503 (D2d will populate). Today this is treated as 404 because
//!    push isn't implemented.
//! 4. For pull mode: synthesize via `agent_card::synthesize_pull_card`,
//!    sign via `agent_card::sign_card` with the active key from
//!    `KeyStore::ensure_active_key`, return JSON.
//!
//! Caching: short browser cache (5 min), longer CDN cache (1 hr).
//! Capability rows or registration metadata changing should bust
//! the cache, but for v1 we accept up-to-1-hr staleness on synthesized
//! cards (cards under push will be invalidated by the refresh job in
//! D2e). Future iteration: add an ETag derived from the signature.

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chakramcp_shared::error::ApiError;

use crate::agent_card::{
    cache_card_for_agent,
    keys::KeyStore,
    sign_card,
    synthesizer::{synthesize_pull_card, AgentRowForSynthesis, CapabilityRowForSynthesis},
    AgentCard, CacheError, CachedCardEnvelope, Fetcher,
};
use crate::auth::AuthUser;
use crate::state::RelayState;

/// How a signed-in caller relates to the account that owns an agent.
#[derive(Debug, Clone, Copy, Default)]
struct Relation {
    member: bool,
    shares_org: bool,
}

impl Relation {
    async fn of(
        state: &RelayState,
        viewer: Option<&AuthUser>,
        account_id: uuid::Uuid,
    ) -> Result<Self, ApiError> {
        let Some(user) = viewer else {
            return Ok(Self::default());
        };
        Ok(Self {
            member: crate::auth::user_is_member(&state.db, user.user_id, account_id).await?,
            shares_org: crate::auth::shares_org_with_account(&state.db, user.user_id, account_id)
                .await?,
        })
    }

    /// Whether something with this `visibility` (an agent's or a
    /// capability's) shows to this caller.
    fn sees(self, visibility: &str) -> bool {
        match visibility {
            "network" => true,
            "org" => self.member || self.shares_org,
            _ => self.member,
        }
    }
}

/// `Cache-Control` and `Vary` for a card response: shared caches may keep
/// what a signed-out caller sees, never what a signed-in one does.
fn cache_headers(signed_in: bool, public: &'static str) -> [(header::HeaderName, &'static str); 2] {
    [
        (
            header::CACHE_CONTROL,
            if signed_in {
                "private, no-store"
            } else {
                public
            },
        ),
        (header::VARY, "Authorization"),
    ]
}

/// Handler for `GET /agents/{account_slug}/{agent_slug}/.well-known/agent-card.json`.
pub async fn get_agent_card(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    // Hard-fail when the v2 discovery surface is gated off.
    if !state.config.discovery_v2_enabled {
        return not_found("agent card service not enabled");
    }

    // Who's asking. No token: signed out. A token that doesn't
    // authenticate is a 401, so a client notices its session is gone
    // rather than silently seeing less.
    let viewer = match headers.get(header::AUTHORIZATION) {
        None => None,
        Some(value) => match crate::auth::authenticate(&state, value.to_str().ok()).await {
            Some(user) => Some(user),
            None => return ApiError::Unauthorized.into_response(),
        },
    };
    let signed_in = viewer.is_some();

    // (1) Resolve account.
    let account = match sqlx::query!(
        r#"
        SELECT id
          FROM accounts
         WHERE slug = $1
           AND tombstoned_at IS NULL
        "#,
        account_slug,
    )
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("account not found"),
        Err(e) => {
            tracing::error!(error = %e, account_slug = %account_slug, "account lookup failed");
            return internal_error();
        }
    };
    let relation = match Relation::of(&state, viewer.as_ref(), account.id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = ?e, "card viewer lookup failed");
            return internal_error();
        }
    };

    // (1b) Slug-alias resolution: if the requested agent_slug was
    // previously renamed, 301-redirect to the new slug so external
    // bookmarks survive (per discovery design §"Rename redirects").
    // Multiple renames collapse to the head of the chain via repeated
    // resolution — bounded at 8 hops to defend against pathological
    // cycles.
    if let Some(target) = match resolve_alias(&state.db, account.id, &agent_slug).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "slug alias lookup failed");
            return internal_error();
        }
    } {
        // Redirect only to an agent this caller may see: the new slug of a
        // private agent is as private as the agent.
        let renamed = match sqlx::query_scalar!(
            r#"SELECT visibility FROM agents
                WHERE account_id = $1 AND slug = $2 AND tombstoned_at IS NULL"#,
            account.id,
            target,
        )
        .fetch_optional(&state.db)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "renamed agent lookup failed");
                return internal_error();
            }
        };
        if !renamed.is_some_and(|v| relation.sees(&v)) {
            return not_found("agent not found");
        }
        let location = format!(
            "/agents/{}/{}/.well-known/agent-card.json",
            account_slug, target
        );
        let [cache_control, vary] = cache_headers(signed_in, "public, max-age=300");
        return (
            StatusCode::MOVED_PERMANENTLY,
            [(header::LOCATION, location.as_str()), cache_control, vary],
            Json(serde_json::json!({
                "moved_to_slug": target,
                "code": "chk.target.renamed",
            })),
        )
            .into_response();
    }

    // (2) Resolve agent. We deliberately fetch tombstoned rows too,
    // so we can serve a clean 410 Gone with a stable error code
    // (chk.target.tombstoned) instead of a 404 — callers with a
    // stale URL learn the shape from the wire, not from us.
    let agent_row = match sqlx::query!(
        r#"
        SELECT id, slug, display_name, description, mode, agent_card_cached,
               tombstoned_at, visibility
          FROM agents
         WHERE account_id = $1
           AND slug       = $2
        "#,
        account.id,
        agent_slug,
    )
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("agent not found"),
        Err(e) => {
            tracing::error!(error = %e, "agent lookup failed");
            return internal_error();
        }
    };
    // An agent this caller may not see is "not found", deleted or not, so
    // its existence isn't revealed.
    if !relation.sees(&agent_row.visibility) {
        return not_found("agent not found");
    }
    if agent_row.tombstoned_at.is_some() {
        return tombstoned_response();
    }

    // (3) Build the unsigned card. Two paths:
    //
    //   - push: the upstream-fetched, normalized card. Already has
    //     our relay URL substituted in supported_interfaces and our
    //     security scheme. May already carry an upstream signature.
    //     If we don't have it cached yet, lazy-fetch on this request
    //     (refresh job in D2e keeps it fresh thereafter).
    //   - pull: synthesize from registration data + capability rows.
    let mut card = match agent_row.mode.as_str() {
        "push" => match cached_or_fetch_push_card(
            &state,
            agent_row.id,
            agent_row.agent_card_cached.as_ref(),
        )
        .await
        {
            Ok(c) => c,
            Err(PushCardError::Unreachable) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(header::CONTENT_TYPE, "application/json")],
                    Json(serde_json::json!({
                        "error": "upstream Agent Card endpoint unreachable",
                        "code": "chk.target.unreachable",
                    })),
                )
                    .into_response()
            }
            Err(PushCardError::Internal) => return internal_error(),
        },
        _ => {
            // Pull-mode synthesis, from the capabilities this caller may
            // see.
            let capabilities = match sqlx::query!(
                r#"
                SELECT id, name, description, visibility
                  FROM agent_capabilities
                 WHERE agent_id = $1
                "#,
                agent_row.id,
            )
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows
                    .into_iter()
                    .filter(|r| relation.sees(&r.visibility))
                    .map(|r| CapabilityRowForSynthesis {
                        id: r.id.to_string(),
                        name: r.name,
                        description: r.description,
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    tracing::error!(error = %e, "capability lookup failed");
                    return internal_error();
                }
            };

            let agent_input = AgentRowForSynthesis {
                account_slug: account_slug.clone(),
                agent_slug: agent_row.slug.clone(),
                display_name: agent_row.display_name.clone(),
                description: agent_row.description.clone(),
                // v0.1.0 default — the agent's own semver. When the
                // agent-version column gets added, source it from there.
                agent_version: "0.1.0".to_string(),
            };

            match synthesize_pull_card(&agent_input, &capabilities, &state.config.relay_base_url) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "synthesis failed");
                    return internal_error();
                }
            }
        }
    };

    // (5) Sign with the active key.
    let key_store = KeyStore::new(state.db.clone());
    let signing_key = match key_store.ensure_active_key().await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(error = %e, "ensure_active_key failed");
            return internal_error();
        }
    };
    if let Err(e) = sign_card(&mut card, &signing_key) {
        tracing::error!(error = %e, "sign_card failed");
        return internal_error();
    }

    // (6) Return JSON with cache headers. Signed out: 5-minute browser
    // cache, 1-hour CDN cache. The signing key rotation cadence (90d, with
    // 30d overlap) means cached cards remain verifiable far longer than
    // this TTL, so we err toward freshness. Signed in: not cached.
    let [cache_control, vary] = cache_headers(signed_in, "public, max-age=300, s-maxage=3600");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            cache_control,
            vary,
        ],
        Json(card),
    )
        .into_response()
}

fn not_found(msg: &'static str) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

/// Resolve a slug rename. Returns Some(head_slug) if the input
/// slug is an alias whose target should be redirected to.
/// Returns None if the slug isn't an alias (live agent or
/// genuinely missing). Walks rename chains transitively up to a
/// bounded depth to collapse to the head — A→B→C returns C, not B.
async fn resolve_alias(
    db: &sqlx::PgPool,
    account_id: uuid::Uuid,
    starting_slug: &str,
) -> Result<Option<String>, sqlx::Error> {
    let mut cur = starting_slug.to_string();
    for _ in 0..8 {
        let row = sqlx::query!(
            r#"SELECT new_slug FROM slug_aliases
                WHERE scope = 'agent'
                  AND account_id = $1
                  AND old_slug = $2
                  AND expires_at > now()"#,
            account_id,
            cur,
        )
        .fetch_optional(db)
        .await?;
        match row {
            Some(r) if r.new_slug == cur => break, // cycle / self-alias, defensive
            Some(r) => cur = r.new_slug,
            None => break,
        }
    }
    if cur == starting_slug {
        Ok(None)
    } else {
        Ok(Some(cur))
    }
}

/// 410 Gone with stable code so generic A2A clients distinguish
/// "this slug is dead, don't retry" from a transient 404.
fn tombstoned_response() -> Response {
    (
        StatusCode::GONE,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({
            "error": "agent tombstoned (slug retired)",
            "code": "chk.target.tombstoned",
        })),
    )
        .into_response()
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
}

/// Outcomes the handler distinguishes for push-mode card retrieval.
enum PushCardError {
    Unreachable,
    Internal,
}

/// Return the cached normalized card if present; otherwise lazy-fetch
/// it via the Fetcher and cache for next time. The refresh job (D2e)
/// keeps push agents' caches fresh proactively, so this lazy path is
/// only the first-request slow case.
async fn cached_or_fetch_push_card(
    state: &RelayState,
    agent_id: uuid::Uuid,
    cached: Option<&serde_json::Value>,
) -> Result<AgentCard, PushCardError> {
    if let Some(value) = cached {
        match serde_json::from_value::<CachedCardEnvelope>(value.clone()) {
            Ok(env) => return Ok(env.normalized),
            Err(e) => {
                // The cache row is corrupt — log and fall through to
                // fetching afresh, which overwrites it.
                tracing::warn!(error = %e, "cached envelope unparseable; refetching");
            }
        }
    }
    let fetcher = Fetcher::new();
    match cache_card_for_agent(&state.db, &fetcher, agent_id, &state.config.relay_base_url).await {
        Ok(card) => Ok(card),
        Err(CacheError::NotPushMode) | Err(CacheError::NotFound) => {
            // Should be unreachable from this code path — the caller
            // already verified mode='push' and the agent is live —
            // but treat defensively.
            Err(PushCardError::Internal)
        }
        Err(CacheError::Fetch(e)) => {
            tracing::warn!(error = %e, agent_id = %agent_id, "upstream card fetch failed");
            Err(PushCardError::Unreachable)
        }
        Err(CacheError::Db(e)) => {
            tracing::error!(error = %e, "cache_card_for_agent DB error");
            Err(PushCardError::Internal)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests against a real sqlx-test DB + the assembled
    //! axum router. Each test seeds account/agent/capability rows,
    //! constructs the router via `crate::router()`, then oneshots an
    //! HTTP request to the /.well-known/agent-card.json path. Asserts
    //! against the parsed AgentCard and verifies the embedded JWS
    //! signature with the active key in JWKS.
    //!
    //! These tests double as the spec-conformance harness: every
    //! D2c-served card flows through this exact path, so changes to
    //! synthesis or signing will be caught here.
    use crate::agent_card::{verify_card, AgentCard, KeyStore};
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn config_with_v2_enabled(enabled: bool, db_url: String) -> SharedConfig {
        SharedConfig {
            database_url: db_url,
            jwt_secret: "test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: enabled,
            log_filter: "warn".into(),
        }
    }

    async fn seed_pull_agent(
        pool: &PgPool,
        account_slug: &str,
        agent_slug: &str,
    ) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
        let acct_id = uuid::Uuid::now_v7();
        let agent_id = uuid::Uuid::now_v7();
        let cap_id = uuid::Uuid::now_v7();

        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Test Account', 'individual')"#,
            acct_id,
            account_slug,
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, description, visibility)
               VALUES ($1, $2, $3, 'Alice Scheduler', 'Returns slots.', 'network')"#,
            agent_id,
            acct_id,
            agent_slug,
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, 'propose_slots', 'Propose meeting slots.',
                       '{"type":"object"}'::jsonb, '{"type":"object"}'::jsonb, 'network')"#,
            cap_id,
            agent_id,
        )
        .execute(pool)
        .await
        .unwrap();

        (acct_id, agent_id, cap_id)
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_signed_card_for_pull_agent(pool: PgPool) {
        let (_acct, _agent, cap_id) = seed_pull_agent(&pool, "acme-corp", "alice").await;

        let cfg = config_with_v2_enabled(true, "ignored-during-test".into());
        let state = crate::state::RelayState::new(pool.clone(), cfg);
        let app = crate::router(state);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/alice/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert!(res
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("max-age=300"));

        let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
        let card: AgentCard = serde_json::from_slice(&body_bytes).unwrap();

        // Identity + URL
        assert_eq!(card.name, "Alice Scheduler");
        assert_eq!(card.description, "Returns slots.");
        assert_eq!(card.supported_interfaces.len(), 1);
        assert_eq!(
            card.supported_interfaces[0].url,
            "http://localhost:8090/agents/acme-corp/alice/a2a/jsonrpc"
        );

        // Skill carries the capability UUID as the spec id.
        assert_eq!(card.skills.len(), 1);
        assert_eq!(card.skills[0].id, cap_id.to_string());
        assert_eq!(card.skills[0].name, "propose_slots");

        // Bearer JWT scheme present.
        assert!(card.security_schemes.contains_key("chakramcp_bearer"));

        // Has exactly one signature, and it verifies against the
        // active key from JWKS — this is the production-correctness
        // assertion that D2a + D2b are wired correctly through D2c.
        assert_eq!(card.signatures.len(), 1);
        let store = KeyStore::new(pool);
        let pub_keys = store.jwks_keys().await.unwrap();
        verify_card(&card, &pub_keys).expect("signature must verify against published JWKS");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_404_when_v2_disabled(pool: PgPool) {
        seed_pull_agent(&pool, "acme-corp", "alice").await;
        let cfg = config_with_v2_enabled(false, "ignored".into());
        let state = crate::state::RelayState::new(pool, cfg);
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/alice/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_404_for_unknown_account(pool: PgPool) {
        let cfg = config_with_v2_enabled(true, "ignored".into());
        let state = crate::state::RelayState::new(pool, cfg);
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/no-such-account/alice/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_404_for_unknown_agent_under_known_account(pool: PgPool) {
        seed_pull_agent(&pool, "acme-corp", "alice").await;
        let cfg = config_with_v2_enabled(true, "ignored".into());
        let state = crate::state::RelayState::new(pool, cfg);
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/no-such-agent/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_410_gone_when_agent_is_tombstoned(pool: PgPool) {
        let (_, agent_id, _) = seed_pull_agent(&pool, "acme-corp", "alice").await;
        sqlx::query!(
            "UPDATE agents SET tombstoned_at = now() WHERE id = $1",
            agent_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        let cfg = config_with_v2_enabled(true, "ignored".into());
        let state = crate::state::RelayState::new(pool, cfg);
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/alice/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::GONE);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["code"], "chk.target.tombstoned");
    }

    /// Spin up a tiny axum upstream that serves an Agent Card body at
    /// `/.well-known/agent-card.json`. Returns the URL to register
    /// against and a handle that keeps the server alive until dropped.
    async fn start_test_upstream(body: serde_json::Value) -> String {
        use axum::http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
        use axum::response::IntoResponse;
        use axum::routing::get;
        let body = std::sync::Arc::new(body.to_string());
        async fn handler(
            axum::extract::State(body): axum::extract::State<std::sync::Arc<String>>,
        ) -> axum::response::Response {
            let mut h = axum::http::HeaderMap::new();
            h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            (StatusCode::OK, h, (*body).clone()).into_response()
        }
        let app = axum::Router::new()
            .route("/.well-known/agent-card.json", get(handler))
            .with_state(body);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{}/.well-known/agent-card.json", addr)
    }

    async fn seed_push_agent(
        pool: &PgPool,
        account_slug: &str,
        agent_slug: &str,
        upstream_url: &str,
    ) -> uuid::Uuid {
        let acct_id = uuid::Uuid::now_v7();
        let agent_id = uuid::Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Test Account', 'individual')"#,
            acct_id,
            account_slug,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, mode, agent_card_url, visibility)
               VALUES ($1, $2, $3, 'Test Push Agent', 'push', $4, 'network')"#,
            agent_id,
            acct_id,
            agent_slug,
            upstream_url,
        )
        .execute(pool)
        .await
        .unwrap();
        agent_id
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn push_mode_lazy_fetches_normalizes_and_signs(pool: PgPool) {
        // Upstream declares a non-bearer scheme + multiple interfaces;
        // the published card must replace both with our canonical
        // chakramcp_bearer + single relay-pointing JSONRPC interface,
        // and verify against our active key.
        let upstream_url = start_test_upstream(serde_json::json!({
            "name": "Travel Planner",
            "description": "Plans trips.",
            "supported_interfaces": [
                { "url": "https://travel.example.com/a2a/v1",
                  "protocol_binding": "JSONRPC",
                  "protocol_version": "0.3" },
                { "url": "https://travel.example.com/a2a/grpc",
                  "protocol_binding": "GRPC",
                  "protocol_version": "0.3" }
            ],
            "version": "2.1.0",
            "capabilities": { "streaming": true },
            "security_schemes": { "upstream_oauth":
                { "oauth2": { "flows": { "client_credentials": {} } } } },
            "security_requirements": [{ "upstream_oauth": ["read"] }],
            "default_input_modes": ["application/json"],
            "default_output_modes": ["application/json"],
            "skills": [{
                "id": "plan-trip", "name": "Plan Trip",
                "description": "Plans an itinerary.", "tags": ["travel"]
            }],
            "signatures": [
                { "protected": "upstream-protected", "signature": "upstream-signature" }
            ]
        }))
        .await;

        let _agent_id = seed_push_agent(&pool, "acme-corp", "travel-planner", &upstream_url).await;

        let cfg = config_with_v2_enabled(true, "ignored".into());
        let state = crate::state::RelayState::new(pool.clone(), cfg);
        let app = crate::router(state);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/travel-planner/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let card: AgentCard = serde_json::from_slice(&body).unwrap();

        // URL: ours, not upstream's.
        assert_eq!(card.supported_interfaces.len(), 1);
        assert_eq!(
            card.supported_interfaces[0].url,
            "http://localhost:8090/agents/acme-corp/travel-planner/a2a/jsonrpc"
        );
        // Auth: ours, not upstream's.
        assert!(card.security_schemes.contains_key("chakramcp_bearer"));
        assert!(!card.security_schemes.contains_key("upstream_oauth"));

        // Pass-through fields preserved.
        assert_eq!(card.name, "Travel Planner");
        assert_eq!(card.version, "2.1.0");
        assert_eq!(card.skills.len(), 1);
        assert_eq!(card.skills[0].id, "plan-trip");

        // Two signatures: upstream's preserved + ours added.
        assert_eq!(card.signatures.len(), 2);
        let store = KeyStore::new(pool);
        let pub_keys = store.jwks_keys().await.unwrap();
        verify_card(&card, &pub_keys).expect("our signature should verify");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn push_mode_returns_503_when_upstream_unreachable(pool: PgPool) {
        // Point at a port nothing is listening on — fetch fails fast.
        let unreachable = "http://127.0.0.1:1/.well-known/agent-card.json";
        seed_push_agent(&pool, "acme-corp", "broken", unreachable).await;

        let cfg = config_with_v2_enabled(true, "ignored".into());
        let state = crate::state::RelayState::new(pool, cfg);
        let app = crate::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/broken/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "chk.target.unreachable");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn jwks_endpoint_returns_keys_after_first_card_request(pool: PgPool) {
        // First request mints the active key as a side effect of
        // signing — verify the JWKS endpoint shows it.
        seed_pull_agent(&pool, "acme-corp", "alice").await;
        let cfg = config_with_v2_enabled(true, "ignored".into());
        let state = crate::state::RelayState::new(pool, cfg);
        let app = crate::router(state);

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/agents/acme-corp/alice/.well-known/agent-card.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/jwks.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let jwks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let keys = jwks["keys"].as_array().unwrap();
        assert_eq!(
            keys.len(),
            1,
            "active key from card sign should appear in JWKS"
        );
        assert_eq!(keys[0]["alg"], "EdDSA");
        assert_eq!(keys[0]["crv"], "Ed25519");
    }

    // ─── Who may see a card ─────────────────────────────────────

    /// A user with a token, and their individual account.
    async fn seed_user(pool: &PgPool, slug: &str) -> (uuid::Uuid, uuid::Uuid, String) {
        let user_id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, 'U', 'x')",
        )
        .bind(user_id)
        .bind(format!("{user_id}@t.local"))
        .execute(pool)
        .await
        .unwrap();
        let account_id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
             VALUES ($1, $2, 'Acct', 'individual', $3)",
        )
        .bind(account_id)
        .bind(slug)
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
        join(pool, account_id, user_id).await;
        let token = chakramcp_shared::jwt::encode_jwt(
            &chakramcp_shared::jwt::UserClaims::new(
                user_id,
                format!("{user_id}@t.local"),
                false,
                1,
            ),
            "test-secret",
        )
        .unwrap();
        (user_id, account_id, token)
    }

    async fn join(pool: &PgPool, account_id: uuid::Uuid, user_id: uuid::Uuid) {
        sqlx::query(
            "INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'member')",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(account_id)
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(
        pool: &PgPool,
        account_id: uuid::Uuid,
        slug: &str,
        visibility: &str,
    ) -> uuid::Uuid {
        let id = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO agents (id, account_id, slug, display_name, description, visibility)
             VALUES ($1, $2, $3, 'Agent', 'Does things.', $4)",
        )
        .bind(id)
        .bind(account_id)
        .bind(slug)
        .bind(visibility)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn add_capability(pool: &PgPool, agent_id: uuid::Uuid, name: &str, visibility: &str) {
        sqlx::query(
            "INSERT INTO agent_capabilities (id, agent_id, name, description, visibility)
             VALUES ($1, $2, $3, 'A capability.', $4)",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(agent_id)
        .bind(name)
        .bind(visibility)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The status, `Cache-Control` and body of a card request.
    async fn fetch(
        pool: &PgPool,
        path: &str,
        token: Option<&str>,
    ) -> (StatusCode, String, serde_json::Value) {
        let app = crate::router(crate::state::RelayState::new(
            pool.clone(),
            config_with_v2_enabled(true, "ignored".into()),
        ));
        let mut req = Request::builder().uri(path);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let cache = res
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        if status == StatusCode::OK {
            assert_eq!(res.headers()[header::VARY], "Authorization");
        }
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            cache,
            serde_json::from_slice(&body).unwrap_or_default(),
        )
    }

    fn card_path(agent: &str) -> String {
        format!("/agents/acme/{agent}/.well-known/agent-card.json")
    }

    fn skill_names(card: &serde_json::Value) -> Vec<String> {
        let mut names: Vec<String> = card["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort();
        names
    }

    /// Private and org-visible agents' cards are 404 signed out and to a
    /// stranger, and served, uncached, to a member of the account.
    #[sqlx::test(migrations = "../migrations")]
    async fn private_and_org_cards_are_for_members(pool: PgPool) {
        let (_, acme, owner) = seed_user(&pool, "acme").await;
        let (_, _, stranger) = seed_user(&pool, "stranger").await;
        seed_agent(&pool, acme, "secret", "private").await;
        seed_agent(&pool, acme, "team", "org").await;

        for agent in ["secret", "team"] {
            let (status, _, _) = fetch(&pool, &card_path(agent), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{agent} signed out");
            let (status, _, _) = fetch(&pool, &card_path(agent), Some(&stranger)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{agent} to a stranger");
            let (status, cache, _) = fetch(&pool, &card_path(agent), Some(&owner)).await;
            assert_eq!(status, StatusCode::OK, "{agent} to its owner");
            assert_eq!(cache, "private, no-store");
        }
    }

    /// An org-visible agent's card shows to someone sharing an org with its
    /// owner; a private one still doesn't.
    #[sqlx::test(migrations = "../migrations")]
    async fn org_cards_show_to_org_members(pool: PgPool) {
        let (owner_id, acme, _) = seed_user(&pool, "acme").await;
        let (colleague_id, _, colleague) = seed_user(&pool, "colleague").await;
        let guild = uuid::Uuid::now_v7();
        sqlx::query(
            "INSERT INTO accounts (id, slug, display_name, account_type) VALUES ($1, 'guild', 'Guild', 'organization')",
        )
        .bind(guild)
        .execute(&pool)
        .await
        .unwrap();
        join(&pool, guild, owner_id).await;
        join(&pool, guild, colleague_id).await;
        seed_agent(&pool, acme, "secret", "private").await;
        seed_agent(&pool, acme, "team", "org").await;

        let (status, _, _) = fetch(&pool, &card_path("team"), Some(&colleague)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = fetch(&pool, &card_path("secret"), Some(&colleague)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// A public card lists network-visible capabilities only; the owner's
    /// view lists them all.
    #[sqlx::test(migrations = "../migrations")]
    async fn public_cards_list_network_capabilities_only(pool: PgPool) {
        let (_, acme, owner) = seed_user(&pool, "acme").await;
        let agent = seed_agent(&pool, acme, "open", "network").await;
        add_capability(&pool, agent, "everyone", "network").await;
        add_capability(&pool, agent, "colleagues", "org").await;
        add_capability(&pool, agent, "internal", "private").await;

        let (status, cache, card) = fetch(&pool, &card_path("open"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache, "public, max-age=300, s-maxage=3600");
        assert_eq!(skill_names(&card), ["everyone"]);

        let (_, _, card) = fetch(&pool, &card_path("open"), Some(&owner)).await;
        assert_eq!(skill_names(&card), ["colleagues", "everyone", "internal"]);
    }

    /// A token that doesn't authenticate is a 401, not a signed-out view.
    #[sqlx::test(migrations = "../migrations")]
    async fn a_bad_token_is_401(pool: PgPool) {
        let (_, acme, _) = seed_user(&pool, "acme").await;
        seed_agent(&pool, acme, "open", "network").await;
        let (status, _, _) = fetch(&pool, &card_path("open"), Some("not-a-token")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// A private agent's old name doesn't redirect to its new one, and a
    /// deleted private agent is "not found", not "gone".
    #[sqlx::test(migrations = "../migrations")]
    async fn renamed_and_deleted_private_agents_stay_hidden(pool: PgPool) {
        let (_, acme, owner) = seed_user(&pool, "acme").await;
        seed_agent(&pool, acme, "renamed", "private").await;
        sqlx::query(
            "INSERT INTO slug_aliases (scope, account_id, old_slug, new_slug, expires_at)
             VALUES ('agent', $1, 'before', 'renamed', now() + interval '90 days')",
        )
        .bind(acme)
        .execute(&pool)
        .await
        .unwrap();
        let deleted = seed_agent(&pool, acme, "deleted", "private").await;
        sqlx::query("UPDATE agents SET tombstoned_at = now() WHERE id = $1")
            .bind(deleted)
            .execute(&pool)
            .await
            .unwrap();

        let (status, _, _) = fetch(&pool, &card_path("before"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = fetch(&pool, &card_path("before"), Some(&owner)).await;
        assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
        let (status, _, _) = fetch(&pool, &card_path("deleted"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = fetch(&pool, &card_path("deleted"), Some(&owner)).await;
        assert_eq!(status, StatusCode::GONE);
    }
}
