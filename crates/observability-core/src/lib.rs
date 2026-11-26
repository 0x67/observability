#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]
//! Runtime metrics gate plus hot-path telemetry helpers.
//!
//! Dependency leaf: `metrics` facade + `hdrhistogram` (+ optional `tokio` flusher) only.
//! No `opentelemetry`, no `tracing-subscriber`, so a feed recv loop can pull it without
//! dragging in backend crates. Backend wiring lives in the `observability` crate.

#[cfg(feature = "flush-tokio")]
mod flush;
mod gate;
mod handoff;
mod sample;
mod timer;

pub mod field;

#[cfg(feature = "flush-tokio")]
pub use flush::spawn_flusher;
pub use gate::{metrics_enabled, set_metrics_enabled};
pub use handoff::{count_msg, drain_all, drain_count, merge_local, record_latency_ns, take_count};
pub use sample::{SAMPLE_1_IN_8192, should_sample};
pub use timer::{Timer, timer};

/// Serializes unit tests that flip the gate or touch the shared slots; both are process-wide.
#[cfg(test)]
static TEST_STATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes [`TEST_STATE`], recovering it after a failed test poisoned it.
#[cfg(test)]
fn test_state() -> std::sync::MutexGuard<'static, ()> {
    TEST_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
