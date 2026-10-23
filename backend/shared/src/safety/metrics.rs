//! Safety metrics (spec §5): what screening found and did, and how long it
//! took, by channel.

use std::time::Duration;

use metrics::{counter, describe_counter, describe_histogram, histogram, Unit};

use super::{Action, Channel, Finding, FindingKind};
use crate::telemetry::names;

/// The actions a kind can come with: contacts are redacted or flagged,
/// questions block or are flagged (the non-English band), the rest are
/// flagged.
fn actions(kind: FindingKind) -> &'static [Action] {
    use FindingKind as K;
    match kind {
        K::Email | K::Phone | K::Url | K::MessagingLink | K::Handle | K::OffPlatform => {
            &[Action::Redacted, Action::Flagged]
        }
        K::Opaque | K::Unjudged => &[Action::Flagged],
        _ => &[Action::Blocked, Action::Flagged],
    }
}

/// Describe the series and register every possible one at 0, so a panel
/// shows 0 rather than nothing.
pub fn register() {
    describe_counter!(
        names::SAFETY_FINDINGS_TOTAL,
        "Message safety findings by channel, kind and action."
    );
    describe_histogram!(
        names::SAFETY_SCREEN_SECONDS,
        Unit::Seconds,
        "Message safety screening time, Jev included, by channel."
    );
    for channel in Channel::ALL {
        for kind in FindingKind::ALL {
            for action in actions(kind) {
                counter!(
                    names::SAFETY_FINDINGS_TOTAL,
                    "channel" => channel.as_str(),
                    "kind" => kind.as_str(),
                    "action" => action.as_str()
                )
                .increment(0);
            }
        }
    }
}

/// Count a screening's findings and its duration.
pub fn record(channel: Channel, findings: &[Finding], elapsed: Duration) {
    for f in findings {
        counter!(
            names::SAFETY_FINDINGS_TOTAL,
            "channel" => channel.as_str(),
            "kind" => f.kind.as_str(),
            "action" => f.action.as_str()
        )
        .increment(1);
    }
    histogram!(names::SAFETY_SCREEN_SECONDS, "channel" => channel.as_str())
        .record(elapsed.as_secs_f64());
}
