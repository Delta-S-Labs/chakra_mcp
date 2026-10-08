//! The JSON log format, end to end through `instrument()`.
//!
//! This test has its own binary, so its own process, because tracing caches
//! each callsite's interest process-wide. While a single scoped subscriber is
//! alive, tracing-core takes the interest of a callsite registered for the
//! first time from the registering thread's default subscriber. Next to the
//! other telemetry tests, a parallel test could create the request span first,
//! from a thread with no subscriber, and cache it as never-enabled. Then this
//! test's log line lost its span (13 runs in 20 on a many-core machine). Keep
//! any test added here under a capturing subscriber, or give it its own file.

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use axum::Router;
use chakramcp_shared::telemetry::{instrument, json_subscriber, REQUEST_ID_HEADER};
use tower::ServiceExt;
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
