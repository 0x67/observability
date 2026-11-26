//! Process-wide metrics gate: one atomic that every thread reads directly.

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-wide metrics gate. Starts off; flip with [`set_metrics_enabled`].
static METRICS_ON: AtomicBool = AtomicBool::new(false);

/// Flip the process-wide gate. Relaxed: the gate is advisory and orders nothing else.
pub fn set_metrics_enabled(on: bool) {
    METRICS_ON.store(on, Ordering::Relaxed);
}

/// Hot-path gate check. The flag is written rarely, so it stays cached on every core and this
/// load costs the same as a thread-local read, with no per-thread copy that could go stale.
#[inline]
pub fn metrics_enabled() -> bool {
    METRICS_ON.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread};

    use super::*;

    /// A toggle reaches a thread that already read the gate, with no call in between.
    // @lat: [[tests#Runtime gate#Toggle reaches other threads without refresh]]
    #[test]
    fn toggle_reaches_other_threads_without_refresh() {
        let _state = crate::test_state();
        set_metrics_enabled(false);

        let (read_tx, read_rx) = mpsc::channel();
        let (flip_tx, flip_rx) = mpsc::channel::<()>();
        let reader = thread::spawn(move || {
            read_tx
                .send(metrics_enabled())
                .expect("main thread waits for the first read");
            flip_rx.recv().expect("main thread signals after the flip");
            metrics_enabled()
        });

        let before = read_rx.recv().expect("reader sends its first read");
        set_metrics_enabled(true);
        flip_tx.send(()).expect("reader waits for the flip");
        let after = reader.join().expect("reader thread panicked");
        set_metrics_enabled(false);

        assert!(!before, "gate must read off before the flip");
        assert!(after, "reader must see the flip with no refresh call");
    }
}
