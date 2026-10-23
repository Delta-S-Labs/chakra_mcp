//! Consent metrics: in-memory counter and gauge updates, shared by the
//! relay (holding, expiry, the approvals API) and the app (the built-in
//! approval page).

use metrics::{counter, gauge, histogram};

use crate::telemetry::names;

/// Count `n` gate-1 requests reaching `outcome` (one of
/// `crate::telemetry::CONSENT_OUTCOMES`).
pub fn record(outcome: &'static str, n: u64) {
    record_kind("request", outcome, n);
}

/// Count `n` consent requests of `kind` (`request` or `reply`) reaching
/// `outcome`.
pub fn record_kind(kind: &'static str, outcome: &'static str, n: u64) {
    if n > 0 {
        counter!(names::CONSENT_REQUESTS_TOTAL, "kind" => kind, "outcome" => outcome).increment(n);
    }
}

/// An invocation finished by a confirmed reply: the same series as the
/// relay's terminal outcomes (`mode=pull`).
pub fn record_finished(status: &str, elapsed_ms: i32) {
    let status = match status {
        "succeeded" => "succeeded",
        "failed" => "failed",
        _ => return,
    };
    counter!(names::INVOCATIONS_TOTAL, "mode" => "pull", "status" => status).increment(1);
    histogram!(names::INVOCATION_DURATION_SECONDS, "mode" => "pull")
        .record(f64::from(elapsed_ms.max(0)) / 1000.0);
}

/// Count `n` held invocations a decision rejected (denied, or no longer
/// authorized). Same series as the relay's terminal outcomes.
pub fn record_rejected(n: u64) {
    if n > 0 {
        counter!(names::INVOCATIONS_TOTAL, "mode" => "pull", "status" => "rejected").increment(n);
    }
}

/// Count `n` invocations that ended unanswered (a reply rejected or not
/// confirmed in time): the outcome counter only, so they don't skew the
/// latency histogram.
pub fn record_ended(status: &'static str, n: u64) {
    if n > 0 {
        counter!(names::INVOCATIONS_TOTAL, "mode" => "pull", "status" => status).increment(n);
    }
}

/// Requests waiting for a decision, as the expiry worker last counted.
pub fn set_pending(n: i64) {
    gauge!(names::CONSENT_PENDING).set(n as f64);
}
