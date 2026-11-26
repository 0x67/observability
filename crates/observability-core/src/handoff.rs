//! Thread-local bounded histogram + cross-thread drain.
//! Bounds 1ns..60s, 3 sig figs. saturating_record clamps tail spikes
//! instead of dropping the sample.
//!
//! Per-thread count. Hot path (count_msg) is a plain Cell, never
//! a shared atomic. Cross-thread aggregation rides merge_local into a shared
//! AtomicU64 slot per thread; drain_count sums them, mirroring the histogram
//! REGISTRY hand-off so a flusher on another thread sees the real total.

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use hdrhistogram::Histogram;

use crate::gate::metrics_enabled;

const HIST_LOW_NS: u64 = 1;
const HIST_HIGH_NS: u64 = 60_000_000_000;
const HIST_SIGFIGS: u8 = 3;

fn new_bounded_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(HIST_LOW_NS, HIST_HIGH_NS, HIST_SIGFIGS)
        .expect("fixed bounds 1ns..60s at 3 sigfigs are always valid")
}

/// Recover a poisoned mutex instead of panicking. Telemetry must survive a panicked holder.
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// One thread's histogram, shared so the flusher thread can drain it.
type SharedHistogram = Arc<Mutex<Histogram<u64>>>;

/// Merge targets, one slot per thread that has called [`merge_local`] at least once.
static REGISTRY: LazyLock<Mutex<Vec<SharedHistogram>>> = LazyLock::new(|| Mutex::new(Vec::new()));

thread_local! {
    static LOCAL: RefCell<Histogram<u64>> = RefCell::new(new_bounded_histogram());
    static LOCAL_SHARED: RefCell<Option<SharedHistogram>> = const { RefCell::new(None) };
}

/// Record one latency sample (ns) into this thread's local histogram. No-op if gate off.
#[inline]
pub fn record_latency_ns(ns: u64) {
    if !metrics_enabled() {
        return;
    }
    LOCAL.with_borrow_mut(|h| h.saturating_record(ns));
}

/// Merge this thread's local histogram into its registry slot, then clear local.
/// First call per thread registers a fresh shared histogram into [`REGISTRY`].
pub fn merge_local() {
    let shared = LOCAL_SHARED.with_borrow_mut(|slot| {
        slot.get_or_insert_with(|| {
            let hist = Arc::new(Mutex::new(new_bounded_histogram()));
            lock_recover(&REGISTRY).push(Arc::clone(&hist));
            hist
        })
        .clone()
    });
    LOCAL.with_borrow_mut(|local| {
        lock_recover(&shared).add(&*local).ok();
        local.reset();
    });

    // hand off message count into this thread's shared slot, same tick
    let count = COUNT.with(|c| c.replace(0));
    let shared_count = COUNT_SHARED.with_borrow_mut(|slot| {
        slot.get_or_insert_with(|| {
            let counter = Arc::new(AtomicU64::new(0));
            lock_recover(&COUNT_REGISTRY).push(Arc::clone(&counter));
            counter
        })
        .clone()
    });
    shared_count.fetch_add(count, Ordering::Relaxed);
}

/// Drain every registered thread histogram into one snapshot, clearing each in place.
/// Only sees data already handed off via [`merge_local`]; a thread that never merges
/// never contributes here.
pub fn drain_all() -> Histogram<u64> {
    let mut merged = new_bounded_histogram();
    for hist in lock_recover(&REGISTRY).iter() {
        let mut guard = lock_recover(hist);
        merged.add(&*guard).ok();
        guard.reset();
    }
    merged
}

/// Message-count slots, one per thread that has merged at least once.
static COUNT_REGISTRY: LazyLock<Mutex<Vec<Arc<AtomicU64>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
    static COUNT_SHARED: RefCell<Option<Arc<AtomicU64>>> = const { RefCell::new(None) };
}

/// Increment this thread's message counter. Wrapping: overflow wraps, never panics.
#[inline]
pub fn count_msg() {
    COUNT.with(|c| c.set(c.get().wrapping_add(1)));
}

/// Read this thread's counter, then reset it to zero. Per-thread primitive;
/// for cross-thread totals use merge_local on the worker tick plus drain_count.
#[inline]
pub fn take_count() -> u64 {
    COUNT.with(|c| c.replace(0))
}

/// Drain every thread's merged message count into one sum, zeroing each slot.
/// Only sees counts handed off via merge_local, never a thread's live local count.
pub fn drain_count() -> u64 {
    lock_recover(&COUNT_REGISTRY)
        .iter()
        .map(|slot| slot.swap(0, Ordering::Relaxed))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::set_metrics_enabled;

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

    /// Cross-thread merge/drain plus saturating clamp. Sole owner of REGISTRY and
    /// COUNT_REGISTRY in this test binary: no other test calls merge_local, drain_all,
    /// or drain_count, else counts here would double up against process-wide state.
    // @lat: [[tests#Cross-thread histogram and count aggregation#merge_local and drain_all aggregate across threads with clamp]]
    #[test]
    fn merge_local_and_drain_all_aggregate_across_threads_with_clamp() {
        const KNOWN: [u64; 5] = [10, 1_000, 50_000, 2_000_000, 999_999_999];
        const OVER_BOUND_NS: u64 = 70_000_000_000; // above HIST_HIGH_NS, must clamp not panic
        const MSG_COUNT: u64 = 7;

        let _state = crate::test_state();
        set_metrics_enabled(true);
        std::thread::spawn(|| {
            for &ns in &KNOWN {
                record_latency_ns(ns);
            }
            record_latency_ns(OVER_BOUND_NS);
            for _ in 0..MSG_COUNT {
                count_msg();
            }
            merge_local();
        })
        .join()
        .unwrap();

        let merged = drain_all();
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

        assert_eq!(drain_count(), MSG_COUNT);
    }
}
