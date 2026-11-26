//! Fails when the steady-state hot path calls the allocator.
//!
//! This binary holds one test, so the process-wide gate and slots need no test lock.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

use observability_core::{
    LatencyWindow, SAMPLE_1_IN_8192, count_msg, drain_into, merge_local, metrics_enabled,
    record_latency_ns, set_metrics_enabled, should_sample, take_count,
};

/// The system allocator, counting this thread's `alloc` and `realloc` calls.
struct CountingAlloc;

thread_local! {
    // const and without Drop, so reading it never allocates or registers a destructor
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

fn count_one() {
    // the allocator also runs during thread-local teardown
    let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
}

// SAFETY: every method forwards its arguments to `System` unchanged; counting touches only a
// const thread-local and cannot allocate.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_one();
        // SAFETY: the caller's `GlobalAlloc::alloc` contract passes through to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System` through this allocator with this `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_one();
        // SAFETY: the caller's `GlobalAlloc::realloc` contract passes through to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn allocs() -> usize {
    ALLOCS.with(Cell::get)
}

// @lat: [[tests#Hot-path allocation#Steady-state hot path does not allocate]]
#[test]
fn steady_state_hot_path_does_not_allocate() {
    set_metrics_enabled(true);
    let mut window = LatencyWindow::default();

    // the first record allocates this thread's histogram and the first merge registers its slot
    record_latency_ns(1_000);
    count_msg();
    merge_local();
    let _ = drain_into(&mut window);

    let before = allocs();
    for i in 0..10_000u64 {
        black_box(metrics_enabled());
        record_latency_ns(black_box(1_000 + i));
        count_msg();
        black_box(take_count());
        black_box(should_sample(black_box(SAMPLE_1_IN_8192)));
        merge_local();
        black_box(drain_into(&mut window));
    }
    assert_eq!(
        allocs() - before,
        0,
        "the steady-state hot path called the allocator"
    );
}
