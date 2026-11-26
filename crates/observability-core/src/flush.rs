//! Optional tokio flusher: publishes one latency window per interval through the `metrics` API.

use std::{
    error::Error,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use hdrhistogram::Histogram;
use metrics::{Gauge, Unit};
use tokio::{runtime::Handle, task::JoinHandle, time::MissedTickBehavior};

use crate::handoff::{LatencyWindow, drain_into};

/// Set while a [`Flusher`] is alive, so two loops never split one stream of drains.
static FLUSHER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Why [`spawn_flusher`] did not start a flusher.
#[derive(Debug, PartialEq, Eq)]
pub enum FlusherError {
    /// The calling thread has no current tokio runtime.
    NoRuntime,
    /// `interval` is zero, which a tokio interval rejects.
    ZeroInterval,
    /// A [`Flusher`] from an earlier call is still alive.
    AlreadyRunning,
}

impl fmt::Display for FlusherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoRuntime => "spawn_flusher needs a current tokio runtime",
            Self::ZeroInterval => "spawn_flusher needs a non-zero interval",
            Self::AlreadyRunning => "a flusher is already running",
        })
    }
}

impl Error for FlusherError {}

/// Running flusher task. Dropping it aborts the task and lets a later [`spawn_flusher`] start.
#[must_use = "dropping the Flusher stops the flusher task"]
pub struct Flusher {
    task: JoinHandle<()>,
}

impl Drop for Flusher {
    fn drop(&mut self) {
        self.task.abort();
        FLUSHER_RUNNING.store(false, Ordering::Release);
    }
}

/// Start a task that drains every slot once per `interval` and publishes the window.
///
/// Each tick adds the drained message count to the `msgs_name` counter and sets five gauges:
/// `{latency_name}.p50`, `.p99`, `.p999`, and `.max` in seconds, and `.samples`, the window's
/// sample count. An empty window sets `.samples` to 0 and leaves the other four unchanged.
/// Set `interval` to the scrape or export interval, so each scrape sees one whole window.
///
/// Call it after the global `metrics` recorder is installed (for example after
/// `observability::init`): the metric handles are built once, here, and handles built before
/// the install are no-ops. Worker threads must call `merge_local` on their own tick, since a
/// drain only sees data already handed off.
///
/// # Errors
///
/// [`FlusherError::NoRuntime`] when the calling thread has no current tokio runtime,
/// [`FlusherError::ZeroInterval`] when `interval` is zero, and
/// [`FlusherError::AlreadyRunning`] while a [`Flusher`] from an earlier call is alive.
///
/// # Panics
///
/// When the current runtime was built without its time driver (`enable_time`).
pub fn spawn_flusher(
    latency_name: &'static str,
    msgs_name: &'static str,
    interval: Duration,
) -> Result<Flusher, FlusherError> {
    let runtime = Handle::try_current().map_err(|_| FlusherError::NoRuntime)?;
    if interval.is_zero() {
        return Err(FlusherError::ZeroInterval);
    }
    // built here, not in the task, so a runtime without timers fails at the call site
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    if FLUSHER_RUNNING.swap(true, Ordering::AcqRel) {
        return Err(FlusherError::AlreadyRunning);
    }

    let gauges = WindowGauges::register(latency_name);
    let msgs = metrics::counter!(msgs_name);
    let task = runtime.spawn(async move {
        let mut window = LatencyWindow::default();
        // the first tick completes at once; skip it so every window spans a whole interval
        tick.tick().await;
        loop {
            tick.tick().await;
            msgs.increment(drain_into(&mut window));
            gauges.publish(window.histogram());
        }
    });

    Ok(Flusher { task })
}

/// Gauge handles for one latency window, built once so a tick formats no names.
struct WindowGauges {
    p50: Gauge,
    p99: Gauge,
    p999: Gauge,
    max: Gauge,
    samples: Gauge,
}

impl WindowGauges {
    fn register(name: &'static str) -> Self {
        let gauge = |suffix: &str, unit: Unit, description: &'static str| {
            let key = format!("{name}.{suffix}");
            metrics::describe_gauge!(key.clone(), unit, description);
            metrics::gauge!(key)
        };
        Self {
            p50: gauge(
                "p50",
                Unit::Seconds,
                "median latency in the last flusher window",
            ),
            p99: gauge(
                "p99",
                Unit::Seconds,
                "99th percentile latency in the last flusher window",
            ),
            p999: gauge(
                "p999",
                Unit::Seconds,
                "99.9th percentile latency in the last flusher window",
            ),
            max: gauge(
                "max",
                Unit::Seconds,
                "highest latency in the last flusher window",
            ),
            samples: gauge(
                "samples",
                Unit::Count,
                "latency samples in the last flusher window",
            ),
        }
    }

    fn publish(&self, window: &Histogram<u64>) {
        self.samples.set(window.len() as f64);
        // an idle window must not read as 0 s latency
        if window.is_empty() {
            return;
        }

        self.p50.set(ns_to_secs(window.value_at_quantile(0.50)));
        self.p99.set(ns_to_secs(window.value_at_quantile(0.99)));
        self.p999.set(ns_to_secs(window.value_at_quantile(0.999)));
        self.max.set(ns_to_secs(window.max()));
    }
}

fn ns_to_secs(ns: u64) -> f64 {
    ns as f64 / 1e9
}

#[cfg(test)]
mod tests;
