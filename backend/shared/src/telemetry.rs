//! Metrics, structured logs and request IDs — the backend half of
//! `docs/superpowers/specs/2026-09-29-observability-phase1-design.md`.
//!
//! Everything on the request path stays in memory: a counter increment or a
//! histogram sample in the `metrics` registry, never I/O. Without
//! `METRICS_ADDR` no recorder is installed and every `metrics` macro is a
//! no-op.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use metrics::{
    counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram, Unit,
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

/// Metric names — the catalogue in the design spec (§4.3). Label values
/// always come from bounded sets: route templates, status codes, fixed enums.
pub mod names {
    pub const HTTP_REQUESTS_TOTAL: &str = "chakramcp_http_requests_total";
    pub const HTTP_REQUEST_DURATION_SECONDS: &str = "chakramcp_http_request_duration_seconds";
    pub const INVOCATIONS_TOTAL: &str = "chakramcp_invocations_total";
    pub const INVOCATION_DURATION_SECONDS: &str = "chakramcp_invocation_duration_seconds";
    pub const MCP_TOOL_CALLS_TOTAL: &str = "chakramcp_mcp_tool_calls_total";
    pub const LIMIT_REFUSALS_TOTAL: &str = "chakramcp_limit_refusals_total";
    pub const RATE_LIMITER_ERRORS_TOTAL: &str = "chakramcp_rate_limiter_errors_total";
    pub const CREDITS_QUEUE_DEPTH: &str = "chakramcp_credits_queue_depth";
    pub const CREDITS_ACCOUNTING_RUNS_TOTAL: &str = "chakramcp_credits_accounting_runs_total";
    pub const CREDITS_CHARGES_TOTAL: &str = "chakramcp_credits_charges_total";
    pub const CREDITS_SWITCH_REFRESHES_TOTAL: &str = "chakramcp_credits_switch_refreshes_total";
    pub const CREDITS_SWITCHES_LAST_REFRESH_TIMESTAMP_SECONDS: &str =
        "chakramcp_credits_switches_last_refresh_timestamp_seconds";
    pub const CREDITS_SWITCHES_STALE: &str = "chakramcp_credits_switches_stale";
    pub const CREDITS_STALE_AFTER_SECONDS: &str = "chakramcp_credits_stale_after_seconds";
    pub const CREDITS_BLOCKED_ACCOUNTS: &str = "chakramcp_credits_blocked_accounts";
    pub const DB_POOL_CONNECTIONS: &str = "chakramcp_db_pool_connections";
    pub const DB_POOL_MAX_CONNECTIONS: &str = "chakramcp_db_pool_max_connections";
    pub const BUILD_INFO: &str = "chakramcp_build_info";
}

/// HTTP latency buckets: 5 ms … 30 s.
const HTTP_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// Invocation latency buckets: 50 ms … 30 min (pull-mode and human-in-the-
/// loop invocations can take minutes).
const INVOCATION_BUCKETS: &[f64] = &[
    0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 300.0, 1800.0,
];

/// Histograms buffer samples until upkeep folds them into buckets.
const UPKEEP_INTERVAL: Duration = Duration::from_secs(5);

/// How often the process and pool gauges are sampled.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// `route` label for requests that matched no route, so a scanner can't
/// create a series per path it tries.
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// The request ID header, minted per request and echoed on the response.
pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

// ─── Logs ────────────────────────────────────────────────

/// How log lines are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable lines — the default, for local dev.
    #[default]
    Text,
    /// One JSON object per line, carrying the current span's fields — prod.
    Json,
}

impl LogFormat {
    /// Parse `LOG_FORMAT`: `text` or `json`, any case; unset or empty means
    /// text. Any other value also means text, plus a warning for the caller
    /// to log — a typo in a log setting must never stop the server.
    pub fn parse(raw: Option<&str>) -> (Self, Option<String>) {
        let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
            return (Self::Text, None);
        };
        match value.to_ascii_lowercase().as_str() {
            "text" => (Self::Text, None),
            "json" => (Self::Json, None),
            _ => (
                Self::Text,
                Some(format!(
                    "unknown LOG_FORMAT {value:?} (expected text or json); using text"
                )),
            ),
        }
    }
}

/// Install the global tracing subscriber (`filter` in `RUST_LOG` syntax, in
/// the format `log_format` names — see [`LogFormat::parse`]) and a panic hook
/// that logs panics at ERROR, so they reach the error-log alert.
pub fn init_tracing(filter: &str, log_format: Option<&str>) {
    let (format, warning) = LogFormat::parse(log_format);
    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    match format {
        LogFormat::Text => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init(),
        LogFormat::Json => json_subscriber(filter, std::io::stdout).init(),
    }
    if let Some(warning) = warning {
        tracing::warn!("{warning}");
    }
    install_panic_hook();
}

/// JSON lines: event fields flattened into the object, plus the current
/// span's fields (`request_id`, `method`, `route` inside a request).
fn json_subscriber<W>(filter: EnvFilter, writer: W) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(writer)
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false)
        .finish()
}

/// Log every panic through `tracing` at ERROR, then run the previous hook
/// (which still prints the message and, if enabled, the backtrace).
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_panic(info);
        previous(info);
    }));
}

fn log_panic(info: &std::panic::PanicHookInfo<'_>) {
    let message = info.payload_as_str().unwrap_or("non-string panic payload");
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_default();
    tracing::error!(panic.message = message, panic.location = %location, "panic");
}

// ─── Build ───────────────────────────────────────────────

/// The version this build reports everywhere: `--version`,
/// `chakramcp_build_info`, the MCP `serverInfo` and outgoing user agents.
/// It's the release (`CHAKRAMCP_VERSION`, set by the release and image
/// builds, e.g. `0.2.0` or `edge`), else the crate version.
pub const VERSION: &str = match option_env!("CHAKRAMCP_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The commit this build came from; CD and the release and image builds set
/// `GIT_SHA`.
pub const GIT_SHA: &str = match option_env!("GIT_SHA") {
    Some(sha) => sha,
    None => "unknown",
};

// ─── Metrics ─────────────────────────────────────────────

/// What `chakramcp_build_info` reports.
#[derive(Debug, Clone, Copy)]
pub struct BuildInfo {
    pub version: &'static str,
    pub git_sha: &'static str,
}

impl BuildInfo {
    /// This build: [`VERSION`] and [`GIT_SHA`].
    pub const CURRENT: BuildInfo = BuildInfo {
        version: VERSION,
        git_sha: GIT_SHA,
    };
}

/// The Prometheus exporter with the catalogue's histogram buckets.
fn prometheus_builder() -> PrometheusBuilder {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(names::HTTP_REQUEST_DURATION_SECONDS.into()),
            HTTP_BUCKETS,
        )
        .expect("static buckets are non-empty")
        .set_buckets_for_metric(
            Matcher::Full(names::INVOCATION_DURATION_SECONDS.into()),
            INVOCATION_BUCKETS,
        )
        .expect("static buckets are non-empty")
}

/// Parse `METRICS_ADDR` (`host:port`). Unset or empty means no metrics
/// listener; anything unparseable is an error — an explicit bind address
/// that's wrong shouldn't be ignored silently.
pub fn parse_metrics_addr(raw: Option<&str>) -> Result<Option<SocketAddr>> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<SocketAddr>().with_context(|| {
                format!("METRICS_ADDR {s:?} is not a socket address (e.g. 0.0.0.0:9464)")
            })
        })
        .transpose()
}

/// Install the global metrics recorder and serve `GET /metrics` on `addr`.
/// Call once, at startup, and only when `METRICS_ADDR` is set.
pub async fn install_metrics(addr: SocketAddr, build: BuildInfo) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding the metrics listener on {addr}"))?;
    let handle = prometheus_builder()
        .install_recorder()
        .context("installing the metrics recorder")?;
    describe();
    gauge!(names::BUILD_INFO, "version" => build.version, "git_sha" => build.git_sha).set(1.0);

    let upkeep = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(UPKEEP_INTERVAL);
        loop {
            tick.tick().await;
            upkeep.run_upkeep();
        }
    });

    let app = metrics_router(handle);
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            tracing::error!(?err, "metrics listener exited");
        }
    });
    tracing::info!(%addr, "metrics listener started");
    Ok(())
}

/// `GET /metrics` → the Prometheus text format; anything else 404s.
fn metrics_router(handle: PrometheusHandle) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let handle = handle.clone();
            async move { handle.render() }
        }),
    )
}

/// Every 15 s, sample the process metrics (CPU, memory, file descriptors,
/// threads, start time) and the gauges of `pools`. Start only when metrics
/// are on.
pub fn spawn_sampler(pools: Vec<(&'static str, PgPool)>) {
    let collector = metrics_process::Collector::default();
    collector.describe();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            tick.tick().await;
            collector.collect();
            for (name, pool) in &pools {
                record_pool(name, pool);
            }
        }
    });
}

/// Set the connection gauges for `pool`, labelled `pool=name`.
pub fn record_pool(name: &'static str, pool: &PgPool) {
    let size = f64::from(pool.size());
    let idle = pool.num_idle() as f64;
    gauge!(names::DB_POOL_CONNECTIONS, "pool" => name, "state" => "idle").set(idle);
    gauge!(names::DB_POOL_CONNECTIONS, "pool" => name, "state" => "in_use")
        .set((size - idle).max(0.0));
    gauge!(names::DB_POOL_MAX_CONNECTIONS, "pool" => name)
        .set(f64::from(pool.options().get_max_connections()));
}

fn describe() {
    use names::*;
    describe_counter!(
        HTTP_REQUESTS_TOTAL,
        "HTTP requests by service, method, route template and status."
    );
    describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        Unit::Seconds,
        "Time until the response headers were ready, by service, method and route template."
    );
    describe_counter!(
        INVOCATIONS_TOTAL,
        "Invocations that reached a terminal status, by mode and status."
    );
    describe_histogram!(
        INVOCATION_DURATION_SECONDS,
        Unit::Seconds,
        "Invocation latency by mode (push: round trip to the agent; pull: enqueue to result). Rejections excluded."
    );
    describe_counter!(
        MCP_TOOL_CALLS_TOTAL,
        "MCP tools/call requests by tool and result."
    );
    describe_counter!(
        LIMIT_REFUSALS_TOTAL,
        "Invocations refused (or, in shadow mode, that would have been) by kind."
    );
    describe_counter!(
        RATE_LIMITER_ERRORS_TOTAL,
        "Redis errors that made the rate limiter fail open."
    );
    describe_gauge!(CREDITS_QUEUE_DEPTH, "Credit charges queued for the worker.");
    describe_counter!(
        CREDITS_ACCOUNTING_RUNS_TOTAL,
        "Credits accounting passes by result (skipped: another instance holds the lock)."
    );
    describe_counter!(CREDITS_CHARGES_TOTAL, "Queued invocations charged.");
    describe_counter!(
        CREDITS_SWITCH_REFRESHES_TOTAL,
        "Credit switch refreshes by result."
    );
    describe_gauge!(
        CREDITS_SWITCHES_LAST_REFRESH_TIMESTAMP_SECONDS,
        Unit::Seconds,
        "Unix time of the last successful credit switch refresh."
    );
    describe_gauge!(
        CREDITS_SWITCHES_STALE,
        "1 while the credit switches are stale (nobody is blocked: fail open)."
    );
    describe_gauge!(
        CREDITS_STALE_AFTER_SECONDS,
        Unit::Seconds,
        "How old the switches may get before they count as stale."
    );
    describe_gauge!(
        CREDITS_BLOCKED_ACCOUNTS,
        "Accounts currently out of credits."
    );
    describe_gauge!(DB_POOL_CONNECTIONS, "Database pool connections by state.");
    describe_gauge!(DB_POOL_MAX_CONNECTIONS, "Database pool size limit.");
    describe_gauge!(
        BUILD_INFO,
        "Always 1; labelled with the version and git sha."
    );
}

// ─── HTTP ────────────────────────────────────────────────

/// Wrap a service's router with, outermost first: the request ID, the HTTP
/// metrics, and the per-request span. `service` is the `service` label.
pub fn instrument(router: Router, service: &'static str) -> Router {
    router
        .layer(TraceLayer::new_for_http().make_span_with(request_span))
        .layer(from_fn_with_state(service, http_metrics))
        .layer(from_fn(request_id))
}

/// A method as a label: the standard seven, else `other` — hyper accepts any
/// token as a method, so the raw value would let a scanner mint series.
pub fn normalize_method(method: &Method) -> &'static str {
    match method.as_str() {
        "GET" => "GET",
        "HEAD" => "HEAD",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        _ => "other",
    }
}

fn route_of(req: &Request) -> &str {
    req.extensions()
        .get::<MatchedPath>()
        .map_or(UNMATCHED_ROUTE, MatchedPath::as_str)
}

async fn http_metrics(State(service): State<&'static str>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = normalize_method(req.method());
    let route = route_of(&req).to_owned();
    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    counter!(
        names::HTTP_REQUESTS_TOTAL,
        "service" => service,
        "method" => method,
        "route" => route.clone(),
        "status" => status
    )
    .increment(1);
    histogram!(
        names::HTTP_REQUEST_DURATION_SECONDS,
        "service" => service,
        "method" => method,
        "route" => route
    )
    .record(started.elapsed().as_secs_f64());
    response
}

/// Mint a request ID for every request — replacing any the client sent, so
/// logs never carry client-chosen values — and echo it on the response.
async fn request_id(mut req: Request, next: Next) -> Response {
    let id =
        HeaderValue::try_from(Uuid::now_v7().to_string()).expect("a UUID is a valid header value");
    req.headers_mut().insert(REQUEST_ID_HEADER, id.clone());
    let mut response = next.run(req).await;
    response.headers_mut().insert(REQUEST_ID_HEADER, id);
    response
}

/// The per-request span: every log line a request produces carries these
/// fields in JSON mode. INFO, so it's on at the prod filter.
fn request_span(req: &Request) -> tracing::Span {
    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    tracing::info_span!(
        "http_request",
        request_id,
        method = normalize_method(req.method()),
        route = route_of(req),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::StatusCode;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
    use metrics_util::MetricKind;
    use tower::ServiceExt;

    use super::*;

    #[test]
    fn methods_outside_the_standard_seven_are_other() {
        for (raw, label) in [
            ("GET", "GET"),
            ("HEAD", "HEAD"),
            ("POST", "POST"),
            ("PUT", "PUT"),
            ("PATCH", "PATCH"),
            ("DELETE", "DELETE"),
            ("OPTIONS", "OPTIONS"),
            ("TRACE", "other"),
            ("PROPFIND", "other"),
            ("XYZZY", "other"),
        ] {
            let method = Method::from_bytes(raw.as_bytes()).unwrap();
            assert_eq!(normalize_method(&method), label, "{raw}");
        }
    }

    #[test]
    fn metrics_addr_is_optional_but_must_parse() {
        assert_eq!(parse_metrics_addr(None).unwrap(), None);
        assert_eq!(parse_metrics_addr(Some("  ")).unwrap(), None);
        assert_eq!(
            parse_metrics_addr(Some("0.0.0.0:9464")).unwrap(),
            Some(SocketAddr::from(([0, 0, 0, 0], 9464)))
        );
        assert!(parse_metrics_addr(Some("localhost")).is_err());
    }

    #[test]
    fn log_format_parses_and_falls_back_to_text() {
        assert_eq!(LogFormat::parse(None), (LogFormat::Text, None));
        assert_eq!(LogFormat::parse(Some("")), (LogFormat::Text, None));
        assert_eq!(LogFormat::parse(Some(" JSON ")), (LogFormat::Json, None));
        assert_eq!(LogFormat::parse(Some("text")), (LogFormat::Text, None));
        let (format, warning) = LogFormat::parse(Some("jsno"));
        assert_eq!(format, LogFormat::Text);
        assert!(warning.unwrap().contains("jsno"));
    }

    #[test]
    fn metrics_render_with_the_catalogue_buckets() {
        let recorder = prometheus_builder().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            counter!(names::HTTP_REQUESTS_TOTAL, "service" => "app").increment(1);
            histogram!(names::HTTP_REQUEST_DURATION_SECONDS, "service" => "app").record(0.02);
            histogram!(names::INVOCATION_DURATION_SECONDS, "mode" => "push").record(42.0);
        });
        let text = handle.render();
        assert!(text.contains("chakramcp_http_requests_total{service=\"app\"} 1"));
        // Buckets, not a summary: histogram_quantile needs `_bucket` series.
        assert!(text.contains(
            "chakramcp_http_request_duration_seconds_bucket{service=\"app\",le=\"0.025\"} 1"
        ));
        assert!(text
            .contains("chakramcp_invocation_duration_seconds_bucket{mode=\"push\",le=\"60\"} 1"));
    }

    #[tokio::test]
    async fn metrics_endpoint_serves_the_text_format() {
        let recorder = prometheus_builder().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            gauge!(names::BUILD_INFO, "version" => "1.2.3", "git_sha" => "abc1234").set(1.0);
        });
        let app = metrics_router(handle);

        let ok = app
            .clone()
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(ok.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("chakramcp_build_info{version=\"1.2.3\",git_sha=\"abc1234\"} 1"));

        let other = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(other.status(), StatusCode::NOT_FOUND);
    }

    fn test_router() -> Router {
        let router = Router::new().route("/items/{id}", get(|| async { "ok" }));
        instrument(router, "app")
    }

    /// `chakramcp_http_requests_total` counts by (method, route, status).
    fn request_counts(snapshotter: &Snapshotter) -> Vec<(String, String, String, u64)> {
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| {
                key.kind() == MetricKind::Counter && key.key().name() == names::HTTP_REQUESTS_TOTAL
            })
            .map(|(key, _, _, value)| {
                let label = |name: &str| {
                    key.key()
                        .labels()
                        .find(|l| l.key() == name)
                        .map(|l| l.value().to_owned())
                        .unwrap_or_default()
                };
                let DebugValue::Counter(n) = value else {
                    unreachable!()
                };
                (label("method"), label("route"), label("status"), n)
            })
            .collect()
    }

    #[tokio::test]
    async fn requests_are_labelled_by_route_template_and_scanners_collapse() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let app = test_router();
        for (method, uri) in [
            ("GET", "/items/1"),
            ("GET", "/items/2"),
            ("GET", "/wp-login.php"),
            ("PROPFIND", "/items/3"),
        ] {
            app.clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let mut counts = request_counts(&snapshotter);
        counts.sort();
        assert_eq!(
            counts,
            vec![
                ("GET".into(), "/items/{id}".into(), "200".into(), 2),
                ("GET".into(), UNMATCHED_ROUTE.into(), "404".into(), 1),
                ("other".into(), "/items/{id}".into(), "405".into(), 1),
            ]
        );
    }

    #[tokio::test]
    async fn every_response_carries_a_fresh_request_id() {
        let app = test_router();
        let response = app
            .clone()
            .oneshot(
                Request::get("/items/1")
                    .header(REQUEST_ID_HEADER, "client-chosen")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = response.headers().get(REQUEST_ID_HEADER).unwrap();
        assert_ne!(id, "client-chosen");
        assert!(Uuid::parse_str(id.to_str().unwrap()).is_ok());

        let unmatched = app
            .oneshot(Request::get("/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(unmatched.headers().contains_key(REQUEST_ID_HEADER));
    }

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

    #[tokio::test]
    async fn json_logs_carry_the_request_span() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = json_subscriber(EnvFilter::new("info"), move || writer.clone());
        let _default = tracing::subscriber::set_default(subscriber);

        let router = Router::new().route(
            "/items/{id}",
            get(|| async {
                tracing::info!(answer = 42, "inside the handler");
                "ok"
            }),
        );
        let response = instrument(router, "app")
            .oneshot(Request::get("/items/9").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let id = response.headers()[REQUEST_ID_HEADER]
            .to_str()
            .unwrap()
            .to_owned();

        let out = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        let line = out
            .lines()
            .find(|l| l.contains("inside the handler"))
            .expect("the handler's log line");
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["level"], "INFO");
        assert_eq!(v["answer"], 42, "event fields are flattened");
        assert_eq!(v["span"]["request_id"], id.as_str());
        assert_eq!(v["span"]["route"], "/items/{id}");
        assert_eq!(v["span"]["method"], "GET");
    }

    #[test]
    fn panics_are_logged_at_error() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(log_panic));
            let _ = std::panic::catch_unwind(|| panic!("boom-7f3a"));
            std::panic::set_hook(previous);
        });
        let out = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("\"level\":\"ERROR\""), "{out}");
        assert!(out.contains("boom-7f3a"), "{out}");
        assert!(out.contains("telemetry.rs"), "{out}");
    }
}
