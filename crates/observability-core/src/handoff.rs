//! Per-thread latency and message counts, handed off to one shared slot per thread.
//!
//! The hot path writes only thread-local state: a bounded `hdrhistogram::Histogram` (1ns..60s,
//! `saturating_record` clamps a tail spike instead of dropping it) and a plain `Cell<u64>` count.
//! `merge_local` on a worker tick, or thread exit, moves both into the thread's `Slot`, and
//! [`drain_into`] folds every slot into a caller-owned window. A slot whose thread has exited is
//! removed by the first drain after the exit.

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use hdrhistogram::Histogram;

use crate::gate::metrics_enabled;

const HIST_LOW_NS: u64 = 1;
const HIST_HIGH_NS: u64 = 60_000_000_000;
// buckets about 1% wide; 3,840 counts (30 KB) per histogram
const HIST_SIGFIGS: u8 = 2;

fn new_bounded_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(HIST_LOW_NS, HIST_HIGH_NS, HIST_SIGFIGS)
        .expect("fixed bounds 1ns..60s at 2 sigfigs are always valid")
}

/// Recover a poisoned mutex instead of panicking. Telemetry must survive a panicked holder.
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// One thread's shared hand-off target. Drained by [`drain_into`], retired by its thread on exit.
struct Slot {
    latency_ns: Mutex<Histogram<u64>>,
    msgs: AtomicU64,
    retired: AtomicBool,
}

/// Every registered slot. Locked once per new thread and once per drain.
static SLOTS: Mutex<Vec<Arc<Slot>>> = Mutex::new(Vec::new());

fn register_slot() -> Arc<Slot> {
    let slot = Arc::new(Slot {
        latency_ns: Mutex::new(new_bounded_histogram()),
        msgs: AtomicU64::new(0),
        retired: AtomicBool::new(false),
    });
    lock_recover(&SLOTS).push(Arc::clone(&slot));
    slot
}

struct ThreadState {
    // None until the first sample, so a thread that only merges never allocates a histogram
    latency_ns: Option<Histogram<u64>>,
    slot: Option<Arc<Slot>>,
}

thread_local! {
    static LOCAL: RefCell<ThreadState> =
        const { RefCell::new(ThreadState { latency_ns: None, slot: None }) };
    static MSGS: Cell<u64> = const { Cell::new(0) };
}

impl ThreadState {
    /// Move pending samples and `msgs` into this thread's slot, registering it on first use.
    fn hand_off(&mut self, msgs: u64) {
        let pending = self.latency_ns.as_mut().filter(|h| !h.is_empty());
        if pending.is_none() && msgs == 0 {
            return;
        }

        let slot = self.slot.get_or_insert_with(register_slot);
        if let Some(local) = pending {
            let mut shared = lock_recover(&slot.latency_ns);
            // every histogram comes from new_bounded_histogram, so the ranges match and add cannot fail
            let added = shared.add(&*local);
            debug_assert!(added.is_ok(), "slot histogram rejected a same-bounds add");
            local.reset();
        }
        if msgs != 0 {
            slot.msgs.fetch_add(msgs, Ordering::Relaxed);
        }
    }
}

impl Drop for ThreadState {
    fn drop(&mut self) {
        let msgs = MSGS.try_with(|c| c.replace(0)).unwrap_or(0);
        self.hand_off(msgs);
        if let Some(slot) = &self.slot {
            // pairs with the Acquire load in drain_into
            slot.retired.store(true, Ordering::Release);
        }
    }
}

/// Record one latency sample (ns) into this thread's local histogram. No-op if gate off.
#[inline]
pub fn record_latency_ns(ns: u64) {
    if !metrics_enabled() {
        return;
    }
    // after thread-local teardown the sample is dropped
    let _ = LOCAL.try_with(|state| {
        state
            .borrow_mut()
            .latency_ns
            .get_or_insert_with(new_bounded_histogram)
            .saturating_record(ns);
    });
}

/// Hand this thread's pending samples and message count to its shared slot.
///
/// The first hand-off with data registers the slot; a merge with nothing pending returns without
/// touching any shared state. A thread that exits without merging hands off on exit instead.
pub fn merge_local() {
    let msgs = MSGS.try_with(|c| c.replace(0)).unwrap_or(0);
    // after thread-local teardown the exit hand-off already ran, so there is nothing to merge
    let _ = LOCAL.try_with(|state| state.borrow_mut().hand_off(msgs));
}

/// Latency snapshot filled by [`drain_into`], in nanoseconds.
///
/// Built with the same bounds as every per-thread histogram, so draining into it cannot fail.
pub struct LatencyWindow(Histogram<u64>);

impl LatencyWindow {
    /// The samples from the last [`drain_into`] call.
    #[must_use]
    pub fn histogram(&self) -> &Histogram<u64> {
        &self.0
    }
}

impl Default for LatencyWindow {
    fn default() -> Self {
        Self(new_bounded_histogram())
    }
}

/// Reset `window`, fill it from every slot, and return the drained message count.
///
/// Each slot is cleared as it is read. A slot whose thread has exited is removed after its final
/// hand-off is drained. Only data already handed off is seen, never a thread's live local state.
#[must_use = "the message count is cleared from every slot"]
pub fn drain_into(window: &mut LatencyWindow) -> u64 {
    window.0.reset();
    let mut msgs = 0u64;
    lock_recover(&SLOTS).retain(|slot| {
        // read before draining: once true, the owner's final hand-off is complete and visible
        let retired = slot.retired.load(Ordering::Acquire);
        let mut shared = lock_recover(&slot.latency_ns);
        if !shared.is_empty() {
            let added = window.0.add(&*shared);
            debug_assert!(added.is_ok(), "window rejected a same-bounds add");
            shared.reset();
        }
        msgs = msgs.wrapping_add(slot.msgs.swap(0, Ordering::Relaxed));
        !retired
    });
    msgs
}

/// Increment this thread's message counter. Wrapping: overflow wraps, never panics.
#[inline]
pub fn count_msg() {
    // after thread-local teardown the count is dropped
    let _ = MSGS.try_with(|c| {
        let n = c.get();
        if n == 0 {
            register_exit_hand_off();
        }
        c.set(n.wrapping_add(1));
    });
}

/// Touch `LOCAL` so its destructor hands this thread's count off at exit.
#[cold]
fn register_exit_hand_off() {
    let _ = LOCAL.try_with(|_| {});
}

/// Read this thread's counter, then reset it to zero. Per-thread primitive;
/// for cross-thread totals use merge_local on the worker tick plus drain_into.
#[inline]
pub fn take_count() -> u64 {
    MSGS.try_with(|c| c.replace(0)).unwrap_or(0)
}

#[cfg(test)]
mod tests;
