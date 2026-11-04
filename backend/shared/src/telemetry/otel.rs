//! Traces: OpenTelemetry spans over OTLP/HTTP, the backend half of
//! `docs/superpowers/specs/2026-11-03-tracing-and-prometheus-rules-design.md`
//! (§4).
//!
//! Off unless `OTEL_EXPORTER_OTLP_ENDPOINT` (or `…_TRACES_ENDPOINT`) is set:
//! then there is no OpenTelemetry layer and none of this runs. On, spans are
//! kept in memory and exported from the SDK's own thread through a bounded
//! queue that drops when full, so a request never waits on an export.
//!
//! - A caller's `traceparent` is never the parent: each request starts its
//!   own trace and links the caller's ([`start_server_span`]).
//! - Nothing is propagated outward: [`send`] makes a client span on our
//!   side only.
//! - Spans carry ids, route templates, hosts, statuses and timings, never
//!   bodies, headers, URL paths or queries.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use metrics::counter;
use opentelemetry::propagation::{Extractor, TextMapPropagator};
use opentelemetry::trace::{SpanContext, Status, TraceContextExt, TracerProvider as _};
use opentelemetry::KeyValue;
use opentelemetry_http::{Bytes, HttpClient, HttpError};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracer, SdkTracerProvider,
};
use opentelemetry_sdk::Resource;
use tracing::field::Empty;
use tracing::subscriber::Interest;
use tracing::{Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::filter::{DynFilterFn, Filtered};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use super::names::TRACE_EXPORT_FAILURES_TOTAL;

/// The target of the spans that exist only for traces: the invocation
/// stages and outbound calls. The log formatter skips them, so a line
/// logged inside one still carries the request span's fields.
pub const SPAN_TARGET: &str = "chakramcp::span";

/// The sampled trace's id, on the response: for operators and support.
pub const TRACE_ID_HEADER: HeaderName = HeaderName::from_static("chakramcp-trace-id");

/// Reasons on `chakramcp_trace_export_failures_total`.
pub const EXPORT_FAILURE_REASONS: [&str; 4] = ["status", "timeout", "connect", "request"];

const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_FLUSH: Duration = Duration::from_secs(1);
const WARN_EVERY_SECS: u64 = 60;

// ─── Configuration ───────────────────────────────────────

/// How root spans are sampled. Children follow their (always local) parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplerChoice {
    AlwaysOn,
    AlwaysOff,
    Ratio(f64),
}

/// The standard `OTEL_*` settings this server reads.
#[derive(Debug, Clone, PartialEq)]
pub struct OtelConfig {
    /// The full URL spans are posted to (`…/v1/traces`).
    pub traces_endpoint: String,
    pub service_name: String,
    pub sampler: SamplerChoice,
}

impl OtelConfig {
    /// From the process environment. `None` means tracing is off; the
    /// strings are warnings, to log once logging is up.
    pub fn from_env() -> (Option<Self>, Vec<String>) {
        Self::parse(|key| std::env::var(key).ok())
    }

    /// `get` reads one variable. Empty means unset, as in the standard.
    pub fn parse(get: impl Fn(&str) -> Option<String>) -> (Option<Self>, Vec<String>) {
        let get = |key: &str| {
            get(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let mut warnings = Vec::new();
        let traces_endpoint = match (
            get("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
            get("OTEL_EXPORTER_OTLP_ENDPOINT"),
        ) {
            (Some(url), _) => url,
            (None, Some(base)) => format!("{}/v1/traces", base.trim_end_matches('/')),
            (None, None) => return (None, warnings),
        };
        if let Err(why) = check_endpoint(&traces_endpoint) {
            // The value itself isn't logged: a vendor URL can carry a key.
            warnings.push(format!("the OTLP traces endpoint {why}; tracing is off"));
            return (None, warnings);
        }

        let ratio = match get("OTEL_TRACES_SAMPLER_ARG") {
            None => 1.0,
            Some(raw) => match raw.parse::<f64>() {
                Ok(r) if (0.0..=1.0).contains(&r) => r,
                _ => {
                    warnings.push(format!(
                        "OTEL_TRACES_SAMPLER_ARG {raw:?} isn't a ratio from 0 to 1; using 1.0"
                    ));
                    1.0
                }
            },
        };
        let sampler = match get("OTEL_TRACES_SAMPLER").map(|s| s.to_ascii_lowercase()) {
            None => SamplerChoice::Ratio(ratio),
            // No remote parent is ever continued, so parent-based samplers
            // act as their root sampler.
            Some(name) => match name.trim_start_matches("parentbased_") {
                "always_on" => SamplerChoice::AlwaysOn,
                "always_off" => SamplerChoice::AlwaysOff,
                "traceidratio" => SamplerChoice::Ratio(ratio),
                _ => {
                    warnings.push(format!(
                        "unknown OTEL_TRACES_SAMPLER {name:?} (expected always_on, always_off or \
                         traceidratio); using traceidratio"
                    ));
                    SamplerChoice::Ratio(ratio)
                }
            },
        };
        let service_name = get("OTEL_SERVICE_NAME").unwrap_or_else(|| "chakramcp".to_owned());
        (
            Some(Self {
                traces_endpoint,
                service_name,
                sampler,
            }),
            warnings,
        )
    }
}

fn check_endpoint(url: &str) -> Result<(), &'static str> {
    let uri: http::Uri = url.parse().map_err(|_| "isn't a URL")?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return Err("isn't an http or https URL");
    }
    if uri.host().is_none_or(str::is_empty) {
        return Err("has no host");
    }
    Ok(())
}

// ─── The pipeline ────────────────────────────────────────

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether this process traces. Read on every request, so it's one atomic
/// load; set once by [`Pipeline::start`].
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The tracer provider: the batch processor, its thread and the exporter.
pub struct Pipeline {
    provider: SdkTracerProvider,
}

impl Pipeline {
    /// Build the exporter and turn tracing on. Exports run on `runtime`;
    /// the SDK's thread only waits for them. `environment` is the hosting
    /// mode (`managed`, `self_hosted`).
    pub fn start(
        cfg: &OtelConfig,
        version: &'static str,
        environment: &'static str,
        runtime: tokio::runtime::Handle,
    ) -> anyhow::Result<Self> {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_http_client(ExportClient::new(runtime)?)
            .with_endpoint(&cfg.traces_endpoint)
            .with_timeout(EXPORT_TIMEOUT)
            .build()?;
        let batches = BatchConfigBuilder::default()
            .with_max_queue_size(2048)
            .with_max_export_batch_size(512)
            .with_scheduled_delay(Duration::from_secs(5))
            .build();
        let root = match cfg.sampler {
            SamplerChoice::AlwaysOn => Sampler::AlwaysOn,
            SamplerChoice::AlwaysOff => Sampler::AlwaysOff,
            SamplerChoice::Ratio(r) => Sampler::TraceIdRatioBased(r),
        };
        let provider = SdkTracerProvider::builder()
            .with_span_processor(
                BatchSpanProcessor::builder(exporter)
                    .with_batch_config(batches)
                    .build(),
            )
            .with_sampler(Sampler::ParentBased(Box::new(root)))
            .with_resource(
                Resource::builder_empty()
                    .with_service_name(cfg.service_name.clone())
                    .with_attributes([
                        KeyValue::new("service.version", version),
                        KeyValue::new("deployment.environment.name", environment),
                    ])
                    .build(),
            )
            .build();
        ENABLED.store(true, Ordering::Relaxed);
        Ok(Self { provider })
    }

    pub fn tracer(&self) -> SdkTracer {
        self.provider.tracer("chakramcp")
    }

    /// Export what's queued, for at most a second, after the HTTP servers
    /// have drained (shutdown's whole budget is Docker's 10 seconds).
    pub async fn shutdown(self) {
        let provider = self.provider;
        let flush = tokio::task::spawn_blocking(move || provider.shutdown());
        if tokio::time::timeout(SHUTDOWN_FLUSH, flush).await.is_err() {
            tracing::warn!("trace flush still running at exit; the rest of the queue is dropped");
        }
    }
}

/// [`layer`]'s type: the OpenTelemetry layer behind its filter.
pub type TracesLayer<S> =
    Filtered<tracing_opentelemetry::OpenTelemetryLayer<S, SdkTracer>, DynFilterFn<S>, S>;

/// The OpenTelemetry layer: spans only (events stay in the logs), from this
/// workspace's crates only, so the exporter's own HTTP stack can never be
/// traced into a loop. `RUST_LOG` doesn't apply to it.
///
/// Its spans are "sometimes" enabled, never cached as "always": with an
/// "always" callsite, tracing skips the check that refreshes the per-layer
/// filters' state, and a `log_enabled!` call with no record after it (sqlx
/// makes one per query) leaves this filter's "no" behind for the next span.
/// That dropped every request after the first from traces
/// (tests/tracing_server.rs).
pub fn layer<S>(tracer: SdkTracer) -> TracesLayer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn ours(meta: &tracing::Metadata<'_>) -> bool {
        meta.is_span()
            && *meta.level() <= tracing::Level::INFO
            && meta.target().starts_with("chakramcp")
            // The request span's form for the logs only (a health check)
            // has no `otel.kind`.
            && (meta.name() != "http_request" || meta.fields().field("otel.kind").is_some())
    }
    fn interest(meta: &'static tracing::Metadata<'static>) -> Interest {
        if ours(meta) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }
    let filter = DynFilterFn::new(
        (|meta, _cx| ours(meta))
            as fn(&tracing::Metadata<'_>, &tracing_subscriber::layer::Context<'_, S>) -> bool,
    )
    .with_callsite_filter(interest as fn(&'static tracing::Metadata<'static>) -> Interest);
    tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_location(false)
        .with_threads(false)
        .with_level(false)
        .with_target(false)
        .with_tracked_inactivity(false)
        .with_filter(filter)
}

// ─── The exporter's HTTP client ──────────────────────────

/// The workspace's reqwest 0.12 for the OTLP exporter (whose own clients are
/// reqwest 0.13). The request runs on the server's runtime; the SDK's thread
/// waits on the result. A failure is counted and, at most once a minute,
/// logged at WARN, so a down collector never floods the logs.
#[derive(Debug)]
struct ExportClient {
    http: reqwest::Client,
    runtime: tokio::runtime::Handle,
}

impl ExportClient {
    fn new(runtime: tokio::runtime::Handle) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().timeout(EXPORT_TIMEOUT).build()?;
        Ok(Self { http, runtime })
    }
}

#[async_trait::async_trait]
impl HttpClient for ExportClient {
    async fn send_bytes(
        &self,
        request: http::Request<Bytes>,
    ) -> Result<http::Response<Bytes>, HttpError> {
        let http = self.http.clone();
        let task = self.runtime.spawn(async move {
            let response = http.execute(reqwest::Request::try_from(request)?).await?;
            let status = response.status();
            let headers = response.headers().clone();
            let mut out = http::Response::new(response.bytes().await?);
            *out.status_mut() = status;
            *out.headers_mut() = headers;
            Ok::<_, reqwest::Error>(out)
        });
        match task.await {
            Ok(Ok(response)) => {
                if !response.status().is_success() {
                    export_failed("status");
                }
                // The exporter turns a non-success status into its error.
                Ok(response)
            }
            Ok(Err(err)) => {
                export_failed(if err.is_timeout() {
                    "timeout"
                } else if err.is_connect() {
                    "connect"
                } else {
                    "request"
                });
                Err(Box::new(err))
            }
            Err(err) => {
                export_failed("request");
                Err(Box::new(err))
            }
        }
    }
}

static LAST_WARN_SECS: AtomicU64 = AtomicU64::new(0);
static FAILED_SINCE_WARN: AtomicU64 = AtomicU64::new(0);

fn export_failed(reason: &'static str) {
    counter!(TRACE_EXPORT_FAILURES_TOTAL, "reason" => reason).increment(1);
    let failed = FAILED_SINCE_WARN.fetch_add(1, Ordering::Relaxed) + 1;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let last = LAST_WARN_SECS.load(Ordering::Relaxed);
    if now >= last + WARN_EVERY_SECS
        && LAST_WARN_SECS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        FAILED_SINCE_WARN.store(0, Ordering::Relaxed);
        tracing::warn!(
            failed,
            reason,
            "trace export failing; those spans are dropped"
        );
    }
}

// ─── Spans ───────────────────────────────────────────────

/// Fill in a request's root span (tracing on): link the caller's trace,
/// start the span, and record `trace_id` so the request's log lines carry
/// it. `span` must have been created with an empty `trace_id` field.
pub fn start_server_span(span: &Span, headers: &HeaderMap, method: &str, route: &str) {
    if let Some(caller) = caller_context(headers) {
        span.add_link_with_attributes(caller, vec![KeyValue::new("link.source", "caller")]);
    }
    if let Some(id) = sampled_trace_id(span) {
        span.record("trace_id", id.as_str());
    }
    span.set_attribute("http.request.method", method.to_owned());
    span.set_attribute("http.route", route.to_owned());
}

/// The span's trace id, when it's sampled (an unsampled id leads nowhere).
/// Starts the OpenTelemetry span if it hasn't started yet.
fn sampled_trace_id(span: &Span) -> Option<String> {
    let cx = span.context();
    let sc = cx.span().span_context().clone();
    (sc.is_valid() && sc.is_sampled()).then(|| sc.trace_id().to_string())
}

/// The trace context a caller sent, if it's valid: for a link, never a
/// parent.
fn caller_context(headers: &HeaderMap) -> Option<SpanContext> {
    if !headers.contains_key("traceparent") {
        return None;
    }
    let cx = TraceContextPropagator::new().extract(&HeaderExtractor(headers));
    let sc = cx.span().span_context().clone();
    sc.is_valid().then_some(sc)
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

/// Middleware inside the request span: the response status on the span, an
/// error status on 5xx, and [`TRACE_ID_HEADER`] when the trace is sampled.
/// `instrument()` adds it only with tracing on.
pub async fn on_response(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let span = Span::current();
    let status = response.status();
    span.set_attribute("http.response.status_code", i64::from(status.as_u16()));
    if status.is_server_error() {
        span.set_status(Status::error(""));
    }
    if let Some(id) = sampled_trace_id(&span) {
        if let Ok(value) = HeaderValue::from_str(&id) {
            response.headers_mut().insert(TRACE_ID_HEADER, value);
        }
    }
    response
}

/// A stage of handling a request, as a span of its own (`name` is the span
/// name in traces). Only traces see it; with tracing off it's
/// [`Span::none`]. Attach with `.instrument(stage(..))`, never `.entered()`
/// across an `.await`.
pub fn stage(name: &'static str) -> Span {
    tracing::info_span!(target: SPAN_TARGET, "stage", otel.name = name)
}

/// Set an attribute on a span for traces only (logs never see it). With
/// tracing off it does nothing.
pub fn attribute(span: &Span, key: &'static str, value: impl Into<opentelemetry::Value>) {
    if enabled() {
        span.set_attribute(key, value);
    }
}

/// Send `request` with a client span named `name` (e.g. `push.deliver`),
/// inside the current request's trace. The span records the method, host
/// and status, never the URL's path or query, which can carry an agent's
/// secrets. Outside a request's span (background loops) it adds nothing,
/// and no trace context is ever sent.
pub async fn send(
    request: reqwest::RequestBuilder,
    name: &'static str,
) -> reqwest::Result<reqwest::Response> {
    let (client, request) = request.build_split();
    let request = request?;
    if !enabled() || Span::current().is_none() {
        return client.execute(request).await;
    }
    let span = tracing::info_span!(
        target: SPAN_TARGET,
        "http_client",
        otel.name = name,
        otel.kind = "client",
        http.request.method = request.method().as_str(),
        server.address = request.url().host_str().unwrap_or_default(),
        http.response.status_code = Empty,
        error.type = Empty,
    );
    let result = client.execute(request).instrument(span.clone()).await;
    match &result {
        Ok(response) => {
            let status = response.status();
            span.record("http.response.status_code", status.as_u16());
            if status.is_server_error() {
                span.set_status(Status::error(""));
            }
        }
        Err(err) => {
            span.record(
                "error.type",
                if err.is_timeout() {
                    "timeout"
                } else if err.is_connect() {
                    "connect"
                } else {
                    "request"
                },
            );
            span.set_status(Status::error(""));
        }
    }
    result
}

// ─── Tests' fake collector ───────────────────────────────

/// A fake OTLP/HTTP collector for other crates' tests: it decodes each
/// export and keeps the spans.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::trace::v1::Span as ProtoSpan;
    use prost::Message;

    /// The spans the collector has received, in arrival order.
    #[derive(Clone, Default)]
    pub struct Collector {
        spans: Arc<Mutex<Vec<ProtoSpan>>>,
    }

    impl Collector {
        pub fn spans(&self) -> Vec<ProtoSpan> {
            self.spans.lock().unwrap().clone()
        }

        /// Wait (up to 10 s) until a span matches `pred`; then all spans.
        pub async fn wait_for(&self, pred: impl Fn(&ProtoSpan) -> bool) -> Vec<ProtoSpan> {
            for _ in 0..200 {
                let spans = self.spans();
                if spans.iter().any(&pred) {
                    return spans;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("no matching span arrived; got {:#?}", self.spans());
        }
    }

    /// Serve the collector on a free local port; returns its traces URL.
    pub async fn serve() -> (String, Collector) {
        async fn export(State(c): State<Collector>, body: Bytes) -> &'static str {
            let req = ExportTraceServiceRequest::decode(body).expect("an OTLP protobuf export");
            let mut spans = c.spans.lock().unwrap();
            for rs in req.resource_spans {
                for ss in rs.scope_spans {
                    spans.extend(ss.spans);
                }
            }
            ""
        }
        let collector = Collector::default();
        let app = Router::new()
            .route("/v1/traces", post(export))
            .with_state(collector.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/v1/traces"), collector)
    }

    /// A span's string attribute.
    pub fn attr(span: &ProtoSpan, key: &str) -> Option<String> {
        use opentelemetry_proto::tonic::common::v1::any_value::Value;
        span.attributes
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_ref()?.value.as_ref())
            .map(|v| match v {
                Value::StringValue(s) => s.clone(),
                Value::IntValue(i) => i.to_string(),
                Value::BoolValue(b) => b.to_string(),
                Value::DoubleValue(d) => d.to_string(),
                other => format!("{other:?}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(vars: &[(&str, &str)]) -> (Option<OtelConfig>, Vec<String>) {
        OtelConfig::parse(|key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn unset_or_empty_means_off() {
        assert_eq!(parse(&[]), (None, vec![]));
        assert_eq!(
            parse(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", ""),
                ("OTEL_TRACES_SAMPLER", "always_on")
            ]),
            (None, vec![])
        );
    }

    #[test]
    fn the_base_endpoint_gets_the_traces_path_and_the_traces_one_wins() {
        let (cfg, w) = parse(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://tempo:4318/")]);
        assert!(w.is_empty());
        let cfg = cfg.unwrap();
        assert_eq!(cfg.traces_endpoint, "http://tempo:4318/v1/traces");
        assert_eq!(cfg.service_name, "chakramcp");
        assert_eq!(cfg.sampler, SamplerChoice::Ratio(1.0));

        let (cfg, _) = parse(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://wrong:4318"),
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                "https://otlp.example/v1/traces",
            ),
        ]);
        assert_eq!(
            cfg.unwrap().traces_endpoint,
            "https://otlp.example/v1/traces"
        );
    }

    #[test]
    fn a_malformed_endpoint_warns_without_the_value_and_stays_off() {
        for bad in ["tempo:4318", "ftp://tempo", "http://", "not a url"] {
            let (cfg, w) = parse(&[("OTEL_EXPORTER_OTLP_ENDPOINT", bad)]);
            assert!(cfg.is_none(), "{bad}");
            assert_eq!(w.len(), 1, "{bad}");
            assert!(!w[0].contains(bad), "{}", w[0]);
        }
    }

    #[test]
    fn samplers() {
        let on = [("OTEL_EXPORTER_OTLP_ENDPOINT", "http://t:4318")];
        let with = |extra: &[(&'static str, &'static str)]| {
            let mut vars = on.to_vec();
            vars.extend_from_slice(extra);
            parse(&vars)
        };
        let sampler = |extra| with(extra).0.unwrap().sampler;
        assert_eq!(
            sampler(&[("OTEL_TRACES_SAMPLER", "always_off")]),
            SamplerChoice::AlwaysOff
        );
        assert_eq!(
            sampler(&[("OTEL_TRACES_SAMPLER", "parentbased_always_on")]),
            SamplerChoice::AlwaysOn
        );
        assert_eq!(
            sampler(&[
                ("OTEL_TRACES_SAMPLER", "TraceIdRatio"),
                ("OTEL_TRACES_SAMPLER_ARG", "0.25")
            ]),
            SamplerChoice::Ratio(0.25)
        );
        // The ratio alone applies to the default sampler.
        assert_eq!(
            sampler(&[("OTEL_TRACES_SAMPLER_ARG", "0.1")]),
            SamplerChoice::Ratio(0.1)
        );

        let (cfg, w) = with(&[("OTEL_TRACES_SAMPLER", "jaeger_remote")]);
        assert_eq!(cfg.unwrap().sampler, SamplerChoice::Ratio(1.0));
        assert_eq!(w.len(), 1);
        let (cfg, w) = with(&[("OTEL_TRACES_SAMPLER_ARG", "2")]);
        assert_eq!(cfg.unwrap().sampler, SamplerChoice::Ratio(1.0));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn the_service_name_can_be_set() {
        let (cfg, _) = parse(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://t:4318"),
            ("OTEL_SERVICE_NAME", "chakramcp-eu"),
        ]);
        assert_eq!(cfg.unwrap().service_name, "chakramcp-eu");
    }

    #[test]
    fn a_caller_context_is_read_only_when_valid() {
        let mut h = HeaderMap::new();
        assert!(caller_context(&h).is_none());
        h.insert(
            "traceparent",
            HeaderValue::from_static("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );
        let sc = caller_context(&h).unwrap();
        assert_eq!(
            sc.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert!(sc.is_remote());
        h.insert(
            "traceparent",
            HeaderValue::from_static("00-zz-00f067aa0ba902b7-01"),
        );
        assert!(caller_context(&h).is_none());
    }
}
