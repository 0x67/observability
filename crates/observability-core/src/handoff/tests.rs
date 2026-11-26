use std::{
    sync::{MutexGuard, mpsc},
    thread,
};

use super::*;
use crate::gate::set_metrics_enabled;

/// Locks shared test state, turns the gate on, and drains what earlier tests left in the slots.
fn setup() -> (MutexGuard<'static, ()>, LatencyWindow) {
    let state = crate::test_state();
    set_metrics_enabled(true);
    let mut window = LatencyWindow::default();
    let _ = drain_into(&mut window);
    (state, window)
}

fn slot_count() -> usize {
    lock_recover(&SLOTS).len()
}

/// Runs `work` on `n` threads and joins them, so every thread's exit hand-off has run on return.
fn run_threads(n: usize, work: fn()) {
    let threads: Vec<_> = (0..n).map(|_| thread::spawn(work)).collect();
    for handle in threads {
        handle.join().expect("worker thread panicked");
    }
}

/// A thread that runs its work, then stays alive until `exit`.
struct Parked {
    release: mpsc::Sender<()>,
    handle: thread::JoinHandle<()>,
}

impl Parked {
    fn spawn(work: fn()) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            work();
            ready_tx.send(()).expect("test waits for the work");
            // the test drops `release` to let the thread exit
            let _ = release_rx.recv();
        });
        ready_rx.recv().expect("parked thread finished its work");
        Self { release, handle }
    }

    fn exit(self) {
        drop(self.release);
        self.handle.join().expect("parked thread panicked");
    }
}

/// take_count reads then resets this thread's counter; second read is zero.
// @lat: [[tests#Per-thread message counting#take_count reads then resets]]
#[test]
fn take_count_reads_then_resets() {
    std::thread::spawn(|| {
        const K: u64 = 5;
        for _ in 0..K {
            count_msg();
        }
        assert_eq!(take_count(), K);
        assert_eq!(take_count(), 0, "second read after take must be zero");
    })
    .join()
    .unwrap();
}

/// Cross-thread merge and drain plus the saturating clamp on a sample above the bound.
// @lat: [[tests#Cross-thread histogram and count aggregation#merge_local and drain_into aggregate across threads with clamp]]
#[test]
fn merge_local_and_drain_into_aggregate_across_threads_with_clamp() {
    const KNOWN: [u64; 5] = [10, 1_000, 50_000, 2_000_000, 999_999_999];
    const OVER_BOUND_NS: u64 = 70_000_000_000; // above HIST_HIGH_NS, must clamp not panic
    const MSG_COUNT: u64 = 7;

    let (_state, mut window) = setup();
    run_threads(1, || {
        for &ns in &KNOWN {
            record_latency_ns(ns);
        }
        record_latency_ns(OVER_BOUND_NS);
        for _ in 0..MSG_COUNT {
            count_msg();
        }
        merge_local();
    });

    let msgs = drain_into(&mut window);
    let merged = window.histogram();
    assert_eq!(merged.len(), KNOWN.len() as u64 + 1);
    let p50 = merged.value_at_quantile(0.5);
    assert!((HIST_LOW_NS..=HIST_HIGH_NS).contains(&p50));
    // hdrhistogram buckets: max() reports the occupied bucket's representative value,
    // which can round up past the raw high bound. Compare against that bucket's own
    // upper edge instead, still proving the clamp landed in-range, not at raw 70e9.
    let clamp_bound = merged.highest_equivalent(HIST_HIGH_NS);
    assert!(
        merged.max() <= clamp_bound,
        "saturating_record must clamp tail spike into bound, not panic or overrun"
    );
    assert_eq!(msgs, MSG_COUNT);
}

/// A second drain into the same window holds only what arrived after the first.
// @lat: [[tests#Cross-thread histogram and count aggregation#Drain resets the window before filling]]
#[test]
fn drain_resets_window_before_filling() {
    let (_state, mut window) = setup();
    run_threads(1, || {
        for _ in 0..10 {
            record_latency_ns(1_000);
        }
        merge_local();
    });
    let _ = drain_into(&mut window);
    assert_eq!(window.histogram().len(), 10);

    run_threads(1, || {
        for _ in 0..3 {
            record_latency_ns(2_000);
        }
        merge_local();
    });
    let _ = drain_into(&mut window);
    assert_eq!(
        window.histogram().len(),
        3,
        "second drain must hold only the second batch"
    );
    assert!(window.histogram().min() >= 1_990, "first batch leaked");
}

/// Slots of exited threads stay until the next drain, which reads them and removes them.
// @lat: [[tests#Slot retirement#Exited threads release their slots]]
#[test]
fn exited_threads_release_their_slots() {
    let (_state, mut window) = setup();
    run_threads(50, || {
        record_latency_ns(1_000);
        merge_local();
    });
    assert_eq!(slot_count(), 50, "retired slots wait for the next drain");

    let _ = drain_into(&mut window);
    assert_eq!(window.histogram().len(), 50);
    assert_eq!(slot_count(), 0, "the drain must remove every retired slot");
}

/// A drain keeps the slot of a thread that is still running.
// @lat: [[tests#Slot retirement#Live thread keeps its slot]]
#[test]
fn live_thread_keeps_its_slot() {
    let (_state, mut window) = setup();
    let worker = Parked::spawn(|| {
        record_latency_ns(1_000);
        merge_local();
    });
    let _ = drain_into(&mut window);
    assert_eq!(slot_count(), 1, "a live thread's slot must survive a drain");

    worker.exit();
    let _ = drain_into(&mut window);
    assert_eq!(slot_count(), 0);
}

/// Samples and counts a thread never merged reach the next drain after it exits.
// @lat: [[tests#Slot retirement#Exit hands off unmerged samples]]
#[test]
fn exit_hands_off_unmerged_samples() {
    let (_state, mut window) = setup();
    run_threads(20, || {
        for ns in [1_000, 2_000, 3_000] {
            record_latency_ns(ns);
        }
        for _ in 0..5 {
            count_msg();
        }
    });

    let msgs = drain_into(&mut window);
    assert_eq!(window.histogram().len(), 60);
    assert_eq!(msgs, 100);
    assert_eq!(slot_count(), 0);
}

/// A thread that only counts, and never records or merges, still hands its count off at exit.
// @lat: [[tests#Slot retirement#Count-only thread hands off at exit]]
#[test]
fn count_only_thread_hands_off_at_exit() {
    let (_state, mut window) = setup();
    run_threads(1, || {
        for _ in 0..3 {
            count_msg();
        }
    });

    assert_eq!(drain_into(&mut window), 3);
    assert_eq!(slot_count(), 0);
}

/// A merge with nothing pending registers no slot, during the thread's life or at its exit.
// @lat: [[tests#Slot retirement#Idle merge registers no slot]]
#[test]
fn idle_merge_registers_no_slot() {
    let (_state, _window) = setup();
    let worker = Parked::spawn(|| {
        for _ in 0..3 {
            merge_local();
        }
    });
    assert_eq!(slot_count(), 0, "an idle merge must not register a slot");

    worker.exit();
    assert_eq!(
        slot_count(),
        0,
        "an idle thread must not register a slot at exit"
    );
}

/// A merge with only a message count registers one slot and carries no samples.
// @lat: [[tests#Slot retirement#Count-only merge registers a slot]]
#[test]
fn count_only_merge_registers_a_slot() {
    let (_state, mut window) = setup();
    run_threads(1, || {
        count_msg();
        merge_local();
    });
    assert_eq!(slot_count(), 1);

    let msgs = drain_into(&mut window);
    assert_eq!(msgs, 1);
    assert_eq!(window.histogram().len(), 0);
}

struct CallsAfterTeardown;

impl Drop for CallsAfterTeardown {
    fn drop(&mut self) {
        record_latency_ns(5);
        count_msg();
        merge_local();
    }
}

thread_local! {
    static LATE: CallsAfterTeardown = const { CallsAfterTeardown };
}

/// Calls from a thread-local destructor that runs after `LOCAL` is gone drop their data.
// @lat: [[tests#Slot retirement#Record after teardown does not panic]]
#[test]
fn record_after_teardown_does_not_panic() {
    let (_state, _window) = setup();
    // a panic in a thread-local destructor aborts the test binary instead of failing the join
    thread::spawn(|| {
        LATE.with(|_| {});
        // LOCAL registers after LATE; destructors run in reverse order, so LOCAL goes first
        record_latency_ns(1_000);
    })
    .join()
    .expect("calls after thread-local teardown must not panic");
}
