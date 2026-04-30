//! `GET /agents/<account-slug>/<agent-slug>/.well-known/agent-card.json`
//! and the agent home pages that delegate to it.
//!
//! This is the public, unauthenticated A2A discovery surface. Anyone
//! — friend, stranger, search engine, generic A2A client, LLM
//! autopiloting — can fetch the card. Auth is required only for
//! method calls (the `/a2a/jsonrpc` endpoint, which lands in D5).
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
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::agent_card::{
    keys::KeyStore,
    sign_card,
    synthesizer::{
        synthesize_pull_card, AgentRowForSynthesis, CapabilityRowForSynthesis,
    },
};
use crate::state::RelayState;

/// Handler for `GET /agents/{account_slug}/{agent_slug}/.well-known/agent-card.json`.
pub async fn get_agent_card(
    State(state): State<RelayState>,
    Path((account_slug, agent_slug)): Path<(String, String)>,
) -> Response {
    // Hard-fail when the v2 discovery surface is gated off.
    if !state.config.discovery_v2_enabled {
        return not_found("agent card service not enabled");
    }

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

    // (2) Resolve agent.
    let agent_row = match sqlx::query!(
        r#"
        SELECT id, slug, display_name, description, mode, agent_card_cached
          FROM agents
         WHERE account_id = $1
           AND slug       = $2
           AND tombstoned_at IS NULL
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

    // (3) Push-mode lands in D2d. For now, 503 if we somehow have a
    // push-mode agent without a cached card yet.
    if agent_row.mode == "push" && agent_row.agent_card_cached.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            Json(serde_json::json!({
                "error": "push-mode card fetch not yet implemented",
                "code": "chk.cards.push_mode_pending",
            })),
        )
            .into_response();
    }

    // (4) Pull-mode synthesis. Capabilities feed the skills array.
    let capabilities = match sqlx::query!(
        r#"
        SELECT id, name, description
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
        // v0.1.0 default — the agent's own semver (NOT the A2A
        // protocol version). When agent-version becomes a column on
        // agents, switch this to the row's value.
        agent_version: "0.1.0".to_string(),
    };

    let mut card = match synthesize_pull_card(
        &agent_input,
        &capabilities,
        &state.config.relay_base_url,
    ) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "synthesis failed");
            return internal_error();
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

    // (6) Return JSON with cache headers.
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            // 5-minute browser cache, 1-hour CDN cache. The signing
            // key rotation cadence (90d, with 30d overlap) means
            // cached cards remain verifiable far longer than this
            // TTL, so we err toward freshness.
            (header::CACHE_CONTROL, "public, max-age=300, s-maxage=3600"),
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

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
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
    async fn returns_404_when_agent_is_tombstoned(pool: PgPool) {
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
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
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
        assert_eq!(keys.len(), 1, "active key from card sign should appear in JWKS");
        assert_eq!(keys[0]["alg"], "EdDSA");
        assert_eq!(keys[0]["crv"], "Ed25519");
    }
}

