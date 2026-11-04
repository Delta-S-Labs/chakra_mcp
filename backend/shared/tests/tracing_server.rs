//! Requests through a real server, with the subscriber installed as the
//! server installs it (`init()`, which also bridges the `log` crate): every
//! request is traced, not just the first.
//!
//! A `log_enabled!` check that no record follows (dependencies make them)
//! used to leave the per-layer filters' state behind. The next request span,
//! whose callsite was cached as always enabled, then took the traces layer's
//! stale "no": it was dropped from traces, its stages became traces of their
//! own, and the response had no trace id. Its own binary, like tracing.rs.

use axum::routing::get;
use axum::Router;
use chakramcp_shared::telemetry::otel::test_support::serve;
use chakramcp_shared::telemetry::otel::{
    self, OtelConfig, Pipeline, SamplerChoice, TRACE_ID_HEADER,
};
use chakramcp_shared::telemetry::{instrument, json_subscriber_traced};
use tracing::Instrument;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_request_through_a_server_is_traced() {
    let (traces_url, collector) = serve().await;
    let cfg = OtelConfig {
        traces_endpoint: traces_url,
        service_name: "chakramcp-test".into(),
        sampler: SamplerChoice::Ratio(1.0),
    };
    let pipeline = Pipeline::start(
        &cfg,
        "0.0.0-test",
        "self_hosted",
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    json_subscriber_traced(EnvFilter::new("info"), std::io::sink, pipeline.tracer()).init();

    let router = Router::new().route(
        "/items/{id}",
        get(|| async {
            async { tracing::info!("looked up") }
                .instrument(otel::stage("lookup"))
                .await;
            // Last, as sqlx does after each query: a check, and nothing
            // logged after it.
            let _ = log::log_enabled!(target: "sqlx::query", log::Level::Warn);
            "ok"
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = instrument(router, "relay");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http = reqwest::Client::new();
    let mut ids = Vec::new();
    for i in 0..6 {
        let res = http
            .get(format!("http://{addr}/items/{i}"))
            .send()
            .await
            .unwrap();
        ids.push(
            res.headers()
                .get(TRACE_ID_HEADER)
                .map(|v| v.to_str().unwrap().to_owned()),
        );
        res.bytes().await.unwrap();
    }
    assert!(
        ids.iter().all(Option::is_some),
        "every response names its trace: {ids:?}"
    );

    // Every stage sits under its request: none became a trace of its own.
    collector.wait_for(|s| s.name == "lookup").await;
    let spans = loop {
        let spans = collector.spans();
        if spans.iter().filter(|s| s.name == "lookup").count() >= ids.len() {
            break spans;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let roots = spans.iter().filter(|s| s.parent_span_id.is_empty());
    assert!(
        roots.clone().all(|s| s.name == "GET /items/{id}"),
        "only request spans are roots: {:?}",
        roots.map(|s| &s.name).collect::<Vec<_>>()
    );
    pipeline.shutdown().await;
}
