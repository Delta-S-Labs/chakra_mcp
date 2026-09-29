//! Relay metrics helpers. Names and the label rules live in
//! [`chakramcp_shared::telemetry::names`]; these are in-memory counter and
//! histogram updates, safe on the invocation path.

use chakramcp_shared::telemetry::names;
use metrics::{counter, histogram};

/// How an invocation reached its target agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The relay called the agent's endpoint (A2A push).
    Push,
    /// The agent pulled it from its inbox and posted the result.
    Pull,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Push => "push",
            Mode::Pull => "pull",
        }
    }
}

/// Record `count` invocations that reached terminal `status`. Call it only
/// after the write that moved them there succeeded, with the rows it
/// actually changed — a guarded update that matched nothing records nothing.
///
/// Anything but a terminal status is ignored (the label set stays bounded),
/// and rejections stay out of the latency histogram: they're written with
/// `elapsed_ms = 0` and would drag the percentiles down.
pub fn record_invocation_outcome(mode: Mode, status: &str, elapsed_ms: i64, count: u64) {
    let status = match status {
        "succeeded" => "succeeded",
        "failed" => "failed",
        "timeout" => "timeout",
        "rejected" => "rejected",
        other => {
            tracing::debug!(
                status = other,
                "not a terminal invocation status; not counted"
            );
            return;
        }
    };
    if count == 0 {
        return;
    }
    counter!(names::INVOCATIONS_TOTAL, "mode" => mode.as_str(), "status" => status)
        .increment(count);
    if status != "rejected" {
        let seconds = elapsed_ms.max(0) as f64 / 1000.0;
        histogram!(names::INVOCATION_DURATION_SECONDS, "mode" => mode.as_str())
            .record_many(seconds, usize::try_from(count).unwrap_or(usize::MAX));
    }
}

/// `tool` for a call the dispatcher didn't recognise (or couldn't parse), so
/// the label set stays bounded by the tools the relay actually has.
pub const UNKNOWN_TOOL: &str = "unknown";

/// Record one MCP `tools/call`: `tool` is a name the dispatcher matched (or
/// [`UNKNOWN_TOOL`]), `ok` whether the tool succeeded.
pub fn record_tool_call(tool: &str, ok: bool) {
    counter!(
        names::MCP_TOOL_CALLS_TOTAL,
        "tool" => tool.to_owned(),
        "result" => if ok { "ok" } else { "error" }
    )
    .increment(1);
}

#[cfg(test)]
pub(crate) mod testing {
    //! Read metrics back in tests: hold a [`Recorded`] for the test's
    //! duration. It installs a thread-local recorder, so run on a
    //! current-thread runtime (`#[tokio::test]` / `#[sqlx::test]` do). Unlike
    //! `metrics_util`'s debugging recorder, reading never resets a value.

    use metrics::{
        Counter, Gauge, Histogram, Key, KeyName, LocalRecorderGuard, Metadata, Recorder,
        SharedString, Unit,
    };
    use metrics_util::registry::{AtomicStorage, Registry};

    struct Store(Registry<Key, AtomicStorage>);

    impl Recorder for Store {
        fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
        fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

        fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
            self.0
                .get_or_create_counter(key, |c| Counter::from_arc(c.clone()))
        }

        fn register_gauge(&self, key: &Key, _: &Metadata<'_>) -> Gauge {
            self.0
                .get_or_create_gauge(key, |g| Gauge::from_arc(g.clone()))
        }

        fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
            self.0
                .get_or_create_histogram(key, |h| Histogram::from_arc(h.clone()))
        }
    }

    pub(crate) struct Recorded {
        store: &'static Store,
        _guard: LocalRecorderGuard<'static>,
    }

    impl Recorded {
        pub(crate) fn start() -> Self {
            // Leaked so the guard can be 'static; a few bytes per test.
            let store: &'static Store = Box::leak(Box::new(Store(Registry::atomic())));
            Self {
                store,
                _guard: metrics::set_default_local_recorder(store),
            }
        }

        /// The counter `name` with exactly these labels (0 if never set).
        pub(crate) fn counter(&self, name: &str, labels: &[(&str, &str)]) -> u64 {
            self.store
                .0
                .get_counter_handles()
                .into_iter()
                .find(|(key, _)| matches(key, name, labels))
                .map_or(0, |(_, c)| c.load(std::sync::atomic::Ordering::SeqCst))
        }

        /// The gauge `name` with exactly these labels.
        pub(crate) fn gauge(&self, name: &str, labels: &[(&str, &str)]) -> Option<f64> {
            self.store
                .0
                .get_gauge_handles()
                .into_iter()
                .find(|(key, _)| matches(key, name, labels))
                .map(|(_, g)| f64::from_bits(g.load(std::sync::atomic::Ordering::SeqCst)))
        }

        /// How many samples the histogram `name` with these labels holds.
        pub(crate) fn histogram_count(&self, name: &str, labels: &[(&str, &str)]) -> usize {
            self.store
                .0
                .get_histogram_handles()
                .into_iter()
                .find(|(key, _)| matches(key, name, labels))
                .map_or(0, |(_, h)| h.data().len())
        }
    }

    fn matches(key: &Key, name: &str, labels: &[(&str, &str)]) -> bool {
        let mut have: Vec<(&str, &str)> = key.labels().map(|l| (l.key(), l.value())).collect();
        let mut want = labels.to_vec();
        have.sort_unstable();
        want.sort_unstable();
        key.name() == name && have == want
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Recorded;
    use super::*;

    #[test]
    fn terminal_outcomes_are_counted_and_timed() {
        let m = Recorded::start();
        record_invocation_outcome(Mode::Push, "succeeded", 120, 1);
        record_invocation_outcome(Mode::Push, "timeout", 30_000, 1);
        record_invocation_outcome(Mode::Pull, "failed", 5, 1);

        let invocations = names::INVOCATIONS_TOTAL;
        assert_eq!(
            m.counter(invocations, &[("mode", "push"), ("status", "succeeded")]),
            1
        );
        assert_eq!(
            m.counter(invocations, &[("mode", "push"), ("status", "timeout")]),
            1
        );
        assert_eq!(
            m.counter(invocations, &[("mode", "pull"), ("status", "failed")]),
            1
        );
        let latency = names::INVOCATION_DURATION_SECONDS;
        assert_eq!(m.histogram_count(latency, &[("mode", "push")]), 2);
        assert_eq!(m.histogram_count(latency, &[("mode", "pull")]), 1);
    }

    #[test]
    fn rejections_are_counted_but_not_timed_and_bulk_counts_add_up() {
        let m = Recorded::start();
        record_invocation_outcome(Mode::Pull, "rejected", 0, 3);
        record_invocation_outcome(Mode::Pull, "rejected", 0, 0);

        let rejected = [("mode", "pull"), ("status", "rejected")];
        assert_eq!(m.counter(names::INVOCATIONS_TOTAL, &rejected), 3);
        assert_eq!(
            m.histogram_count(names::INVOCATION_DURATION_SECONDS, &[("mode", "pull")]),
            0
        );
    }

    #[test]
    fn non_terminal_statuses_are_ignored() {
        let m = Recorded::start();
        record_invocation_outcome(Mode::Pull, "pending", 0, 1);
        record_invocation_outcome(Mode::Pull, "in_progress", 0, 1);
        record_invocation_outcome(Mode::Pull, "weird", 0, 1);
        for status in ["pending", "in_progress", "weird"] {
            assert_eq!(
                m.counter(
                    names::INVOCATIONS_TOTAL,
                    &[("mode", "pull"), ("status", status)]
                ),
                0
            );
        }
    }
}
