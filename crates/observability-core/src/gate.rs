//! Runtime gate: process-wide atomic, per-thread mirror to skip the
//! cross-core hop on the hot path once primed.

use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

/// Process-wide metrics gate. Starts OFF; flip with [`set_metrics_enabled`].
pub static METRICS_ON: AtomicBool = AtomicBool::new(false);

thread_local! {
    static GATE_MIRROR: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Flip process-wide gate. Relaxed: gate is advisory, not ordering-sensitive vs other state.
pub fn set_metrics_enabled(on: bool) {
    METRICS_ON.store(on, Ordering::Relaxed);
}

/// Sync this thread's cached gate value from the process-wide flag.
/// Call once per thread at startup, and again after any runtime toggle a thread cares about.
pub fn refresh_thread_gate() {
    GATE_MIRROR.set(Some(METRICS_ON.load(Ordering::Relaxed)));
}

/// Gate check for hot path. Reads primed thread-local mirror; falls to shared atomic if unset.
#[inline]
pub fn metrics_enabled() -> bool {
    GATE_MIRROR
        .with(|mirror| mirror.get())
        .unwrap_or_else(|| METRICS_ON.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gate starts off; set + refresh flips the thread mirror.
    // @lat: [[tests#Runtime gate#Gate defaults off then flips via refresh]]
    #[test]
    fn gate_defaults_off_then_flips_via_refresh() {
        assert!(
            !metrics_enabled(),
            "gate must default off before any enable call"
        );
        set_metrics_enabled(true);
        refresh_thread_gate();
        assert!(metrics_enabled());
        // leave gate clean for any test sharing this process
        set_metrics_enabled(false);
        refresh_thread_gate();
    }
}
