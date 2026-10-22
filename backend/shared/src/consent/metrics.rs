//! Consent metrics: in-memory counter and gauge updates, shared by the
//! relay (holding, expiry, the approvals API) and the app (the built-in
//! approval page).

use metrics::{counter, gauge};

use crate::telemetry::names;

/// Count `n` gate-1 requests reaching `outcome` (one of
/// `crate::telemetry::CONSENT_OUTCOMES`).
pub fn record(outcome: &'static str, n: u64) {
    if n > 0 {
        counter!(names::CONSENT_REQUESTS_TOTAL, "kind" => "request", "outcome" => outcome)
            .increment(n);
    }
}

/// Count `n` held invocations a decision rejected (denied, or no longer
/// authorized). Same series as the relay's terminal outcomes.
pub fn record_rejected(n: u64) {
    if n > 0 {
        counter!(names::INVOCATIONS_TOTAL, "mode" => "pull", "status" => "rejected").increment(n);
    }
}

/// Requests waiting for a decision, as the expiry worker last counted.
pub fn set_pending(n: i64) {
    gauge!(names::CONSENT_PENDING).set(n as f64);
}
