//! Hot-path cost of the `observability-core` calls, std and hdrhistogram only.
//!
//! Each row times 10,000 batches. A batch runs the call 1,000 times back to back (once for the
//! merge and drain rows), and the row reports the batch mean per call, so a call shorter than
//! one clock tick still gets a number. These are service times with no offered rate, so no
//! coordinated-omission correction applies.
//!
//! Run: `cargo bench -p observability-core --bench hot_path`.

use std::{
    hint::black_box,
    process::Command,
    sync::{
        atomic::{Ordering, compiler_fence},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use hdrhistogram::Histogram;
use observability_core::{
    LatencyWindow, SAMPLE_1_IN_8192, count_msg, drain_into, merge_local, metrics_enabled,
    record_latency_ns, set_metrics_enabled, should_sample,
};

const BATCHES: u32 = 10_000;
const NS_BATCH: u32 = 1_000;
const CHURN_ROUNDS: usize = 40;
const CHURN_THREADS: usize = 50;

fn main() {
    set_metrics_enabled(true);

    println!(
        "profile: debug_assertions={} batches={BATCHES}",
        cfg!(debug_assertions)
    );
    let clock = clock_overhead();
    println!(
        "clock overhead, Instant pair (ns): p50={} p99={} max={}",
        clock.value_at_quantile(0.50),
        clock.value_at_quantile(0.99),
        clock.max()
    );
    println!(
        "\n{:<28} {:>6} {:>6} {:>10} {:>10} {:>10} {:>10}",
        "row (batch mean ns/call)", "batch", "n", "p50", "p99", "p999", "max"
    );

    // drain rows run first, so the main thread holds no slot and each row sees exactly n slots
    let mut window = LatencyWindow::default();
    let mut pool = HelperPool::new();
    for slots in [1, 8, 64] {
        pool.grow_to(slots);
        let hist = measure(1, || pool.fill(), || drain(&mut window));
        print_row(&format!("drain, {slots} live slots"), 1, &hist);
    }
    pool.stop();

    let hist = measure(
        NS_BATCH,
        || {},
        || {
            black_box(metrics_enabled());
        },
    );
    print_row("metrics_enabled", NS_BATCH, &hist);

    let mut i = 0u64;
    let hist = measure(
        NS_BATCH,
        || {},
        || {
            i = i.wrapping_add(1);
            record_latency_ns(black_box(1_000 + (i & 1023)));
        },
    );
    print_row("record_latency_ns", NS_BATCH, &hist);

    let hist = measure(
        NS_BATCH,
        || {},
        || {
            count_msg();
            // without the fence the optimizer folds the batch into one add
            compiler_fence(Ordering::SeqCst);
        },
    );
    print_row("count_msg", NS_BATCH, &hist);

    let hist = measure(
        NS_BATCH,
        || {},
        || {
            black_box(should_sample(black_box(SAMPLE_1_IN_8192)));
        },
    );
    print_row("should_sample", NS_BATCH, &hist);

    // clear what the record row left behind, so the idle row merges nothing
    merge_local();
    print_row("merge_local, idle", 1, &measure(1, || {}, merge_local));
    print_row(
        "merge_local, 1 pending",
        1,
        &measure(1, || record_latency_ns(1_000), merge_local),
    );

    churn(&mut window);
}

fn clock_overhead() -> Histogram<u64> {
    let mut hist = bench_histogram();
    for _ in 0..BATCHES {
        let start = Instant::now();
        hist.saturating_record(nanos(start));
    }
    hist
}

/// Times `BATCHES` runs of `batch` back-to-back `call`s; `setup` runs untimed before each.
fn measure(batch: u32, mut setup: impl FnMut(), mut call: impl FnMut()) -> Histogram<u64> {
    let mut hist = bench_histogram();
    for _ in 0..BATCHES {
        setup();
        let start = Instant::now();
        for _ in 0..batch {
            call();
        }
        hist.saturating_record(nanos(start));
    }
    hist
}

fn drain(window: &mut LatencyWindow) {
    black_box(drain_into(window));
}

fn print_row(name: &str, batch: u32, hist: &Histogram<u64>) {
    let per_call = |total: u64| total as f64 / f64::from(batch);
    println!(
        "{name:<28} {batch:>6} {:>6} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
        hist.len(),
        per_call(hist.value_at_quantile(0.50)),
        per_call(hist.value_at_quantile(0.99)),
        per_call(hist.value_at_quantile(0.999)),
        per_call(hist.max()),
    );
}

/// Threads that each own one slot and refill it on request, so a drain sees live data.
struct HelperPool {
    helpers: Vec<(mpsc::Sender<()>, JoinHandle<()>)>,
    ack_tx: mpsc::Sender<()>,
    ack_rx: mpsc::Receiver<()>,
}

impl HelperPool {
    fn new() -> Self {
        let (ack_tx, ack_rx) = mpsc::channel();
        Self {
            helpers: Vec::new(),
            ack_tx,
            ack_rx,
        }
    }

    fn grow_to(&mut self, n: usize) {
        while self.helpers.len() < n {
            let (tx, rx) = mpsc::channel::<()>();
            let ack = self.ack_tx.clone();
            let handle = thread::spawn(move || {
                while rx.recv().is_ok() {
                    record_latency_ns(1_000);
                    merge_local();
                    ack.send(())
                        .expect("bench main thread outlives its helpers");
                }
            });
            self.helpers.push((tx, handle));
        }
    }

    fn fill(&self) {
        for (tx, _) in &self.helpers {
            tx.send(()).expect("helper threads run until stop");
        }
        for _ in &self.helpers {
            self.ack_rx.recv().expect("every helper acks its fill");
        }
    }

    fn stop(self) {
        for (tx, handle) in self.helpers {
            drop(tx);
            handle.join().expect("helper thread panicked");
        }
    }
}

/// Rounds of short-lived recording threads with one drain after each, as a flusher tick would
/// run: shows whether exited threads keep memory and slow the drain.
fn churn(window: &mut LatencyWindow) {
    let before = rss_kib();
    let mut drain_ns = 0;
    for _ in 0..CHURN_ROUNDS {
        let threads: Vec<_> = (0..CHURN_THREADS)
            .map(|_| {
                thread::spawn(|| {
                    record_latency_ns(1_000);
                    merge_local();
                })
            })
            .collect();
        for handle in threads {
            handle.join().expect("churn thread panicked");
        }
        let start = Instant::now();
        drain(window);
        drain_ns = nanos(start);
    }
    let after = rss_kib();

    let threads = CHURN_ROUNDS * CHURN_THREADS;
    println!(
        "\nchurn: {threads} threads in {CHURN_ROUNDS} rounds, each records 1 sample and merges, then exits; one drain per round"
    );
    println!("churn rss before (KiB): {}", show(before));
    println!("churn rss after (KiB):  {}", show(after));
    match (before, after) {
        (Some(b), Some(a)) => println!("churn rss growth (KiB): {}", a as i64 - b as i64),
        _ => println!("churn rss growth (KiB): n/a"),
    }
    println!("churn last drain (ns): {drain_ns}");
}

fn rss_kib() -> Option<u64> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

fn show(kib: Option<u64>) -> String {
    kib.map_or_else(|| "n/a".to_owned(), |v| v.to_string())
}

fn bench_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(1, 60_000_000_000, 3).expect("1 ns..60 s at 3 sigfigs is valid")
}

fn nanos(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}
