//! Counts of telemetry the pipeline lost, by signal, for the guard and the metrics recorder.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::error::Signal;

/// Spans and log records lost since init: a full queue, a failed export, or a full log
/// line buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    pub traces: u64,
    pub logs: u64,
}

/// Why items were dropped; the `reason` label of `observability.dropped`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DropReason {
    /// A bounded export queue was full.
    #[cfg(feature = "otel")]
    QueueFull,
    /// The export of the item's batch returned an error.
    #[cfg(feature = "otel")]
    ExportFailed,
    /// A writer sink's line buffer was full.
    BufferFull,
}

impl DropReason {
    fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "otel")]
            Self::QueueFull => "queue_full",
            #[cfg(feature = "otel")]
            Self::ExportFailed => "export_failed",
            Self::BufferFull => "buffer_full",
        }
    }
}

/// Per-signal totals shared by every sink, processor, and exporter of one pipeline.
#[derive(Clone, Debug, Default)]
pub(crate) struct DropCounters(Arc<Totals>);

#[derive(Debug, Default)]
struct Totals {
    traces: AtomicU64,
    logs: AtomicU64,
}

impl DropCounters {
    /// Add `n` dropped items to the signal's total and to `observability.dropped`. A drop
    /// before a metrics recorder is installed reaches only the total.
    pub(crate) fn add(&self, signal: Signal, reason: DropReason, n: u64) {
        let total = match signal {
            Signal::Traces => &self.0.traces,
            Signal::Logs => &self.0.logs,
        };
        total.fetch_add(n, Ordering::Relaxed);
        metrics::counter!(
            "observability.dropped",
            "signal" => signal.as_str(),
            "reason" => reason.as_str()
        )
        .increment(n);
    }

    pub(crate) fn snapshot(&self) -> Dropped {
        Dropped {
            traces: self.0.traces.load(Ordering::Relaxed),
            logs: self.0.logs.load(Ordering::Relaxed),
        }
    }
}

// the test needs a trace drop reason, which exists only with `otel`
#[cfg(all(test, feature = "otel"))]
mod tests {
    use metrics_util::{
        CompositeKey, MetricKind,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{DropCounters, DropReason, Dropped};
    use crate::error::Signal;

    // the counter's value for one label pair, or None when it was never incremented
    fn dropped_counter(
        counted: &[(CompositeKey, DebugValue)],
        labels: [(&str, &str); 2],
    ) -> Option<u64> {
        counted.iter().find_map(|(key, value)| {
            let key_labels: Vec<(&str, &str)> = key
                .key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect();
            let matches = key.kind() == MetricKind::Counter
                && key.key().name() == "observability.dropped"
                && key_labels == labels;
            match value {
                DebugValue::Counter(n) if matches => Some(*n),
                _ => None,
            }
        })
    }

    // @lat: [[tests#Drop counters#Add updates totals and metric]]
    #[test]
    fn add_updates_totals_and_metric() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let counters = DropCounters::default();

        metrics::with_local_recorder(&recorder, || {
            counters.add(Signal::Traces, DropReason::QueueFull, 2);
            counters.add(Signal::Logs, DropReason::BufferFull, 3);
        });

        assert_eq!(counters.snapshot(), Dropped { traces: 2, logs: 3 });
        let counted: Vec<(CompositeKey, DebugValue)> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, value)| (key, value))
            .collect();
        let traces = [("signal", "traces"), ("reason", "queue_full")];
        let logs = [("signal", "logs"), ("reason", "buffer_full")];
        assert_eq!(dropped_counter(&counted, traces), Some(2), "{counted:?}");
        assert_eq!(dropped_counter(&counted, logs), Some(3), "{counted:?}");
    }
}
