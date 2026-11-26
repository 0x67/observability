//! Optional tokio flusher.

use crate::{
    gate::refresh_thread_gate,
    handoff::{drain_all, drain_count},
};

/// Spawn a 100ms tokio loop draining histograms + counters into `metrics` gauges/counters.
///
/// Gauge quantiles do not re-aggregate across process instances: averaging p99 across
/// processes is wrong math. Fine for single-process feed handler. Worker threads must
/// call `merge_local` on their own tick, since `drain_all`/`drain_count` only see data
/// already handed off to the shared registries, never a thread's live local state.
pub fn spawn_flusher(latency_prefix: &'static str, count_metric: &'static str) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tick.tick().await;
            refresh_thread_gate();
            let h = drain_all();
            metrics::gauge!(format!("{latency_prefix}.p50")).set(h.value_at_quantile(0.50) as f64);
            metrics::gauge!(format!("{latency_prefix}.p99")).set(h.value_at_quantile(0.99) as f64);
            metrics::gauge!(format!("{latency_prefix}.p999"))
                .set(h.value_at_quantile(0.999) as f64);
            metrics::counter!(count_metric).increment(drain_count());
        }
    });
}

// NOTE: flush-tokio flusher (spawn_flusher) not covered here. It only proves
// itself through the metrics facade global recorder, and installing one would
// race every other test sharing this process. Skipped.
