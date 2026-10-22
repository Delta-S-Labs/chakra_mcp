//! Consent metrics: in-memory counter and gauge updates.

use chakramcp_shared::telemetry::names;
use metrics::{counter, gauge};

/// Count `n` gate-1 requests reaching `outcome` (one of
/// `chakramcp_shared::telemetry::CONSENT_OUTCOMES`).
pub fn record(outcome: &'static str, n: u64) {
    if n > 0 {
        counter!(names::CONSENT_REQUESTS_TOTAL, "kind" => "request", "outcome" => outcome)
            .increment(n);
    }
}

/// Requests waiting for a decision, as the expiry worker last counted.
pub fn set_pending(n: i64) {
    gauge!(names::CONSENT_PENDING).set(n as f64);
}
