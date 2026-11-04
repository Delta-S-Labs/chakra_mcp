//! An invocation's trace: `POST /v1/invoke` and the granter's inbox pull,
//! with their stages under the request spans.
//!
//! Its own binary with one global subscriber, as `shared/tests/tracing.rs`
//! explains. The fixture is `seed_demo`'s (handlers/invoke.rs) in runtime
//! queries, so this file adds nothing to `.sqlx`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chakramcp_relay::{router, RelayState};
use chakramcp_shared::config::SharedConfig;
use chakramcp_shared::telemetry::json_subscriber_traced;
use chakramcp_shared::telemetry::otel::test_support::serve;
use chakramcp_shared::telemetry::otel::{OtelConfig, Pipeline, SamplerChoice, TRACE_ID_HEADER};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

fn config() -> SharedConfig {
    SharedConfig {
        database_url: "ignored".into(),
        jwt_secret: "test-secret".into(),
        admin_email: None,
        survey_enabled: false,
        frontend_base_url: "http://localhost:3000".into(),
        app_base_url: "http://localhost:8080".into(),
        relay_base_url: "http://localhost:8090".into(),
        discovery_v2_enabled: false,
        log_filter: "warn".into(),
    }
}

struct Fixture {
    caller_token: String,
    granter_token: String,
    granter_agent: Uuid,
    grantee_agent: Uuid,
    grant: Uuid,
}

/// Two accounts with one agent each, friends, and a grant: Bob's agent may
/// call Alice's `propose_slots`.
async fn seed(pool: &PgPool) -> Fixture {
    let mut tokens = Vec::new();
    let mut users = Vec::new();
    for name in ["Alice", "Bob"] {
        let user = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, 'x')",
        )
        .bind(user)
        .bind(format!("{name}-{user}@t.local"))
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
        let token = format!("ck_trace_{user}");
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix) VALUES ($1, $2, $3, 't', 'ck_trace')",
        )
        .bind(Uuid::now_v7())
        .bind(user)
        .bind(hex::encode(Sha256::digest(token.as_bytes())))
        .execute(pool)
        .await
        .unwrap();
        users.push(user);
        tokens.push(token);
    }
    let mut agents = Vec::new();
    for (user, slug) in users.iter().zip(["alice-scheduler", "bob-caller"]) {
        let account = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id) \
             VALUES ($1, $2, $2, 'individual', $3)",
        )
        .bind(account)
        .bind(format!("{slug}-{account}"))
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'owner')")
            .bind(Uuid::now_v7())
            .bind(account)
            .bind(user)
            .execute(pool)
            .await
            .unwrap();
        let agent = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO agents (id, account_id, slug, display_name, visibility) VALUES ($1, $2, $3, $3, 'network')",
        )
        .bind(agent)
        .bind(account)
        .bind(slug)
        .execute(pool)
        .await
        .unwrap();
        agents.push(agent);
    }
    let (granter_agent, grantee_agent) = (agents[0], agents[1]);
    let capability = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agent_capabilities (id, agent_id, name, description, input_schema, output_schema, visibility) \
         VALUES ($1, $2, 'propose_slots', 'Propose meeting slots.', '{\"type\":\"object\"}'::jsonb, \
                 '{\"type\":\"object\"}'::jsonb, 'network')",
    )
    .bind(capability)
    .bind(granter_agent)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status, decided_at) \
         VALUES ($1, $2, $3, 'accepted', now())",
    )
    .bind(Uuid::now_v7())
    .bind(granter_agent)
    .bind(grantee_agent)
    .execute(pool)
    .await
    .unwrap();
    let grant = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO grants (id, granter_agent_id, grantee_agent_id, capability_id, status) \
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(grant)
    .bind(granter_agent)
    .bind(grantee_agent)
    .bind(capability)
    .execute(pool)
    .await
    .unwrap();
    Fixture {
        caller_token: tokens[1].clone(),
        granter_token: tokens[0].clone(),
        granter_agent,
        grantee_agent,
        grant,
    }
}

#[sqlx::test(migrations = "../migrations")]
async fn an_invoke_and_its_pull_are_traced_stage_by_stage(pool: PgPool) {
    let (traces_url, collector) = serve().await;
    let cfg = OtelConfig {
        traces_endpoint: traces_url,
        service_name: "chakramcp-test".into(),
        sampler: SamplerChoice::AlwaysOn,
    };
    let pipeline = Pipeline::start(
        &cfg,
        "0.0.0-test",
        "self_hosted",
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    tracing::subscriber::set_global_default(json_subscriber_traced(
        EnvFilter::new("warn"),
        std::io::sink,
        pipeline.tracer(),
    ))
    .unwrap();

    let f = seed(&pool).await;
    let app = router(RelayState::new(pool.clone(), config()));
    let call = |req: Request<Body>| {
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (parts, body) = res.into_parts();
            let body = body.collect().await.unwrap().to_bytes();
            (parts.status, parts.headers, body)
        }
    };

    let (status, headers, body) = call(
        Request::post("/v1/invoke")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {}", f.caller_token))
            .body(Body::from(
                serde_json::json!({
                    "grant_id": f.grant,
                    "grantee_agent_id": f.grantee_agent,
                    "input": {"duration_min": 30}
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&body)
    );
    let invoke_trace = headers[TRACE_ID_HEADER].to_str().unwrap().to_owned();

    let (status, headers, _) = call(
        Request::get(format!("/v1/inbox?agent_id={}", f.granter_agent))
            .header(header::AUTHORIZATION, format!("Bearer {}", f.granter_token))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let pull_trace = headers[TRACE_ID_HEADER].to_str().unwrap().to_owned();
    assert_ne!(invoke_trace, pull_trace, "each request is its own trace");

    let spans = collector.wait_for(|s| s.name == "inbox.claim").await;
    let spans = if spans.iter().any(|s| s.name == "POST /v1/invoke") {
        spans
    } else {
        collector.wait_for(|s| s.name == "POST /v1/invoke").await
    };
    let span = |name: &str| {
        spans.iter().find(|s| s.name == name).unwrap_or_else(|| {
            panic!(
                "no {name:?} span in {:?}",
                spans.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        })
    };
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();

    // The invoke: the request span, authentication and the authorize-and-
    // enqueue stage under it, and that stage's own steps under that.
    let root = span("POST /v1/invoke");
    assert_eq!(hex(&root.trace_id), invoke_trace);
    let invoke = span("invoke");
    assert_eq!(span("auth").parent_span_id, root.span_id);
    assert_eq!(invoke.parent_span_id, root.span_id);
    for step in ["invoke.load_grant", "invoke.limits", "invoke.enqueue"] {
        let s = span(step);
        assert_eq!(s.parent_span_id, invoke.span_id, "{step} under invoke");
        assert_eq!(s.trace_id, root.trace_id, "{step}");
    }

    // The pull: the claim, under its request.
    let pull = span("GET /v1/inbox");
    assert_eq!(hex(&pull.trace_id), pull_trace);
    let claim = span("inbox.claim");
    assert_eq!(claim.parent_span_id, pull.span_id);
    let items = claim
        .attributes
        .iter()
        .find(|kv| kv.key == "inbox.items")
        .and_then(|kv| kv.value.as_ref())
        .map(|v| format!("{v:?}"));
    assert!(
        items.is_some_and(|v| v.contains("IntValue(1)")),
        "{:?}",
        claim.attributes
    );

    pipeline.shutdown().await;
}
