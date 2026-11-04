//! Traces end to end: `instrument()` with tracing on, exporting to a fake
//! OTLP collector, plus the JSON logs beside them.
//!
//! Its own binary, with one global subscriber, for the reason in
//! `json_logs.rs`: tracing caches callsite interest process-wide. One test
//! function, because the pipeline exports on the runtime it started on.

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use chakramcp_shared::telemetry::otel::test_support::{attr, serve, Collector};
use chakramcp_shared::telemetry::otel::{
    self, OtelConfig, Pipeline, SamplerChoice, TRACE_ID_HEADER,
};
use chakramcp_shared::telemetry::{instrument, json_subscriber_traced, REQUEST_ID_HEADER};
use http_body_util::BodyExt;
use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use opentelemetry_proto::tonic::trace::v1::status::StatusCode as OtelStatus;
use opentelemetry_proto::tonic::trace::v1::Span as ProtoSpan;
use tower::ServiceExt;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn line(&self, containing: &str) -> serde_json::Value {
        let out = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        let line = out
            .lines()
            .find(|l| l.contains(containing))
            .unwrap_or_else(|| panic!("no log line with {containing:?} in:\n{out}"));
        serde_json::from_str(line).unwrap()
    }
}

/// A stand-in push agent: keeps the headers of the last request it got.
async fn fake_agent() -> (String, Arc<Mutex<Option<HeaderMap>>>) {
    let seen = Arc::new(Mutex::new(None));
    let app = Router::new().route(
        "/hook/{secret}",
        post({
            let seen = seen.clone();
            move |headers: HeaderMap| async move {
                *seen.lock().unwrap() = Some(headers);
                "ok"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn named<'a>(spans: &'a [ProtoSpan], name: &str) -> &'a ProtoSpan {
    spans.iter().find(|s| s.name == name).unwrap_or_else(|| {
        panic!(
            "no span {name:?} in {:#?}",
            spans.iter().map(|s| &s.name).collect::<Vec<_>>()
        )
    })
}

const CALLER_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_are_traced_and_their_logs_carry_the_trace_id() {
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
    assert!(otel::enabled());
    let logs = Captured::default();
    let writer = logs.clone();
    tracing::subscriber::set_global_default(json_subscriber_traced(
        EnvFilter::new("info"),
        move || writer.clone(),
        pipeline.tracer(),
    ))
    .unwrap();

    let (agent_url, agent_saw) = fake_agent().await;
    let http = reqwest::Client::new();
    let router = Router::new()
        .route(
            "/items/{id}",
            get(|| async {
                tracing::info!(answer = 42, "inside the handler");
                async { tracing::info!("inside a stage") }
                    .instrument(otel::stage("lookup"))
                    .await;
                "ok"
            }),
        )
        .route("/boom", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/out",
            get(move || async move {
                let url = format!("{agent_url}/hook/secret-path?token=abc");
                let res = otel::send(http.post(url).body("{}"), "push.deliver")
                    .await
                    .unwrap();
                res.status().as_u16().to_string()
            }),
        );
    let app = instrument(router, "relay");
    // Each response is read to the end and dropped, as a server does: the
    // request's span ends with its body.
    let call = |req: Request<Body>| {
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (parts, body) = res.into_parts();
            body.collect().await.unwrap();
            (parts.status, parts.headers)
        }
    };

    // A request carrying a caller's trace context.
    let (status, headers) = call(
        Request::get("/items/7")
            .header(
                "traceparent",
                format!("00-{CALLER_TRACE}-00f067aa0ba902b7-01"),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trace_id = headers[TRACE_ID_HEADER].to_str().unwrap().to_owned();
    let request_id = headers[REQUEST_ID_HEADER].to_str().unwrap().to_owned();
    assert_eq!(trace_id.len(), 32);
    assert_ne!(
        trace_id, CALLER_TRACE,
        "the caller's trace is never the parent"
    );

    // The other requests, then everything they exported.
    let (_, boom) = call(Request::get("/boom").body(Body::empty()).unwrap()).await;
    assert!(boom.contains_key(TRACE_ID_HEADER));
    let (_, health) = call(Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert!(
        !health.contains_key(TRACE_ID_HEADER),
        "health checks aren't traced"
    );
    let (out, _) = call(Request::get("/out").body(Body::empty()).unwrap()).await;
    assert_eq!(out, StatusCode::OK);
    let spans = wait_for_all(&collector).await;

    // The root span: named by route, a server span, with its status and a
    // link to the caller's trace.
    let root = named(&spans, "GET /items/{id}");
    assert_eq!(hex(&root.trace_id), trace_id);
    assert!(root.parent_span_id.is_empty(), "a root span");
    assert_eq!(root.kind, SpanKind::Server as i32);
    assert_eq!(attr(root, "http.route").as_deref(), Some("/items/{id}"));
    assert_eq!(attr(root, "http.request.method").as_deref(), Some("GET"));
    assert_eq!(
        attr(root, "http.response.status_code").as_deref(),
        Some("200")
    );
    assert_eq!(root.links.len(), 1);
    assert_eq!(hex(&root.links[0].trace_id), CALLER_TRACE);
    assert_eq!(root.links[0].attributes[0].key, "link.source");

    // The stage is its child.
    let stage = named(&spans, "lookup");
    assert_eq!(stage.trace_id, root.trace_id);
    assert_eq!(stage.parent_span_id, root.span_id);

    // A 5xx marks its span as an error.
    let boom = named(&spans, "GET /boom");
    assert_eq!(boom.status.as_ref().unwrap().code, OtelStatus::Error as i32);

    // Health checks have no span.
    assert!(spans.iter().all(|s| !s.name.contains("healthz")));

    // The outbound call: a client span under its request, host only, and no
    // trace context sent to the agent.
    let client = named(&spans, "push.deliver");
    let parent = named(&spans, "GET /out");
    assert_eq!(client.kind, SpanKind::Client as i32);
    assert_eq!(client.parent_span_id, parent.span_id);
    assert_eq!(attr(client, "server.address").as_deref(), Some("127.0.0.1"));
    assert_eq!(
        attr(client, "http.response.status_code").as_deref(),
        Some("200")
    );
    let headers = agent_saw
        .lock()
        .unwrap()
        .clone()
        .expect("the agent got the call");
    assert!(!headers.contains_key("traceparent"));
    assert!(!headers.contains_key("tracestate"));

    // No span carries a URL's path or query.
    for span in &spans {
        for kv in &span.attributes {
            let value = format!("{:?}", kv.value);
            assert!(
                !value.contains("secret-path") && !value.contains("token") && !value.contains('?'),
                "{}: {} = {value}",
                span.name,
                kv.key
            );
        }
    }

    // The logs: the request's lines carry its trace id, a line logged inside
    // a stage included (the formatter skips trace-only spans).
    for message in ["inside the handler", "inside a stage"] {
        let line = logs.line(message);
        assert_eq!(line["span"]["trace_id"], trace_id.as_str(), "{message}");
        assert_eq!(line["span"]["request_id"], request_id.as_str(), "{message}");
        assert_eq!(line["span"]["route"], "/items/{id}", "{message}");
    }

    pipeline.shutdown().await;
}

/// Every span the four requests make, once the batch (every 5 s) arrives.
async fn wait_for_all(collector: &Collector) -> Vec<ProtoSpan> {
    collector.wait_for(|s| s.name == "push.deliver").await;
    collector.wait_for(|s| s.name == "GET /out").await
}
