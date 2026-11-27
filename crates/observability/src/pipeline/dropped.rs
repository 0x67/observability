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
    BufferFull,
}

impl DropReason {
    fn as_str(self) -> &'static str {
        match self {
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

#[cfg(test)]
mod tests {
    use metrics_util::{
        CompositeKey, MetricKind,
        debugging::{DebugValue, DebuggingRecorder},
    };

    use super::{DropCounters, DropReason, Dropped};
    use crate::error::Signal;

    // @lat: [[tests#Drop counters#Add updates totals and metric]]
    #[test]
    fn add_updates_totals_and_metric() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let counters = DropCounters::default();

        metrics::with_local_recorder(&recorder, || {
            counters.add(Signal::Logs, DropReason::BufferFull, 2);
            counters.add(Signal::Logs, DropReason::BufferFull, 1);
        });

        assert_eq!(counters.snapshot(), Dropped { traces: 0, logs: 3 });
        let counted: Vec<(CompositeKey, DebugValue)> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, value)| (key, value))
            .collect();
        let [(key, DebugValue::Counter(3))] = counted.as_slice() else {
            panic!("one counter at 3: {counted:?}");
        };
        assert_eq!(key.kind(), MetricKind::Counter);
        assert_eq!(key.key().name(), "observability.dropped");
        let labels: Vec<(&str, &str)> = key
            .key()
            .labels()
            .map(|label| (label.key(), label.value()))
            .collect();
        assert_eq!(labels, [("signal", "logs"), ("reason", "buffer_full")]);
    }
}
