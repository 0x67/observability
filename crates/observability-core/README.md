# observability-core

The runtime metrics gate plus hot-path telemetry helpers. A dependency leaf: `metrics`,
`hdrhistogram`, and an optional `tokio` flusher, nothing else. No `opentelemetry`, no
`tracing-subscriber`, so a feed receive loop can pull it without dragging in backend crates. All
backend wiring lives one crate up, in [`observability`](../observability).

## Why a separate crate

The hot path writes only thread-local state. Its one shared read is the gate flag, which is
written rarely and so stays in each core's cache. Everything sampled per-message lives in
thread-local state and moves to one shared slot per thread only on a periodic tick or at thread
exit. A flusher on another thread sees only data a worker already handed off, never a worker's
live local state.

## Runtime gate

One private process-wide `AtomicBool`, off at start. Every thread reads it directly, and a
`set_metrics_enabled` call is visible on every thread without per-thread setup or a refresh call.

```rust,ignore
pub fn set_metrics_enabled(on: bool);   // flip the process-wide gate (Relaxed; advisory)
pub fn metrics_enabled() -> bool;       // hot-path check: one Relaxed load
```

`observability::init_metrics` turns the gate on after a successful recorder install, so a
binary rarely calls `set_metrics_enabled` directly.

## Latency, counts, sampling

```rust,ignore
pub struct Timer(/* ... */);
pub fn timer(metric: &'static str) -> Timer;   // RAII; records elapsed secs on drop
pub fn record_latency_ns(ns: u64);             // thread-local histogram; no-op if gate off
pub fn count_msg();                            // per-thread Cell<u64>, wrapping
pub fn take_count() -> u64;                    // read + reset this thread's counter
pub fn should_sample(mask: u64) -> bool;       // 1-in-(mask+1) per-thread sampler
pub const SAMPLE_1_IN_8192: u64;               // mask = 8192 - 1
```

- `timer` reads the clock only when the gate is on; the `None` variant records nothing on drop.
- `record_latency_ns` lands in a thread-local `hdrhistogram::Histogram` bounded 1ns..60s at 2
  significant figures: buckets about 1% wide, 30 KB per histogram. It uses `saturating_record`, so a tail spike clamps into range instead of
  panicking.
- `should_sample` requires `mask == 2^n - 1`. It fires on a fresh thread's very first call, then
  exactly once per `mask + 1` consecutive calls.

```rust
use observability_core::{metrics_enabled, record_latency_ns, count_msg, should_sample, SAMPLE_1_IN_8192};

let t0 = std::time::Instant::now();
// decode ...
count_msg();
record_latency_ns(t0.elapsed().as_nanos() as u64);

if should_sample(SAMPLE_1_IN_8192) {
    // periodic, not every message
}
```

## Cross-thread aggregation

Each thread hands its samples and message count to one shared slot, on a worker tick or when the
thread exits. A reader drains every slot into a window it owns.

```rust,ignore
pub fn merge_local();                                 // hand this thread's samples + count to its slot
pub struct LatencyWindow;                             // Default; histogram() -> &Histogram<u64>, in ns
pub fn drain_into(window: &mut LatencyWindow) -> u64; // reset, fill from every slot, return msg count
```

A thread registers its slot on its first hand-off with data. A merge with nothing pending touches
no shared state. When a thread exits, it hands off whatever it has not merged and retires its
slot, and the first drain after that removes the slot. Short-lived threads therefore do not grow
memory, and no sample is lost at exit.

`drain_into` sees only data already handed off, never a thread's live local state. Reuse one
`LatencyWindow` across drains, so a periodic reader does not allocate. Poisoned mutexes are
recovered rather than propagated, and telemetry survives a panicked holder.

## Optional tokio flusher

```toml
observability-core = { git = "...", tag = "observability-core-v0.2.0", features = ["flush-tokio"] }
```

```rust,no_run
use std::time::Duration;

use observability_core::{Flusher, FlusherError};

// inside a tokio runtime, after `observability::init` or another `metrics` recorder install
fn start_flusher() -> Result<Flusher, FlusherError> {
    let flusher = observability_core::spawn_flusher(
        "feed.decode_latency",
        "feed.msgs",
        Duration::from_secs(15), // the scrape or export interval
    )?;
    // dropping `flusher` stops the task
    Ok(flusher)
}
```

Once per `interval` the flusher drains every slot into one window. It adds the drained message
count to the `feed.msgs` counter and sets five gauges: `feed.decode_latency.p50`, `.p99`,
`.p999`, and `.max` in seconds, plus `.samples`, the number of samples in the window. The
`.p50`, `.p99`, `.p999`, and `.max` gauges are described with `Unit::Seconds`, and `.samples`
with `Unit::Count`. A window with no samples sets `.samples` to 0 and leaves the latency gauges
at their last value, so an idle period never reads as 0 s.

Call `spawn_flusher` after the recorder install. It builds its metric handles once, and handles
built before an install are no-ops. Set `interval` to the scrape or export interval: each scrape
then reads one whole window, and no window falls between two scrapes.

`spawn_flusher` returns `FlusherError::NoRuntime` outside a tokio runtime, `ZeroInterval` for a
zero interval, and `AlreadyRunning` while a `Flusher` from an earlier call is alive. It panics
when the runtime was built without its time driver.

The gauges describe one process. Quantiles do not combine across instances (averaging p99
across processes is wrong math), and the flusher doesn't publish per-bucket counters, so it suits
a single-process service. This is the only feature and the only thing that pulls in `tokio`, and
it enables only tokio's `rt` and `time` features.

## Performance

`cargo bench -p observability-core --bench hot_path --features flush-tokio` times each hot-path
call and prints batch means in ns per call (1,000 calls per batch, or 1 for the drain and merge
rows). The numbers below come from an Apple M4 Pro on macOS 27.2.1 with rustc 1.96.1 in the
bench profile. Before is the code with two registries, a per-thread gate mirror, and 3
significant figures; after is the slot registry, the single-atomic gate, and 2 significant
figures. Each cell shows run 1 / run 2.

| Row                      | Before p50        | Before p99            | After p50        | After p99         |
| ------------------------ | ----------------- | --------------------- | ---------------- | ----------------- |
| drain, 1 live slot       | 14,543 / 13,047   | 23,759 / 64,511       | 875 / 1,000      | 2,833 / 1,917     |
| drain, 8 live slots      | 99,839 / 88,511   | 546,303 / 585,727     | 12,543 / 13,583  | 20,927 / 22,639   |
| drain, 64 live slots     | 681,983 / 683,519 | 2,760,703 / 2,658,303 | 93,887 / 93,631  | 131,839 / 134,783 |
| `metrics_enabled`        | 0.29 / 0.25       | 0.38 / 0.38           | 0.25 / 0.25      | 0.33 / 0.33       |
| `record_latency_ns`      | 1.46 / 1.46       | 2.21 / 2.21           | 2.12 / 2.17      | 2.71 / 2.71       |
| `count_msg`              | 0.96 / 1.00       | 1.21 / 1.29           | 1.12 / 1.12      | 1.42 / 1.42       |
| `should_sample`          | 0.79 / 0.75       | 1.00 / 1.04           | 0.75 / 0.75      | 0.96 / 0.96       |
| `merge_local`, idle      | 1,125 / 959       | 2,959 / 2,583         | under 42 (1 tick) | 42 / 42          |
| `merge_local`, 1 pending | 9,295 / 9,335     | 11,503 / 13,503       | 1,250 / 1,250    | 1,583 / 1,542     |

The churn case runs 40 rounds of 50 short-lived threads that each record one sample and merge,
with one drain per round. Before, resident memory grew by 438,352 and 438,128 KiB, and the last
drain walked all 2,000 slots in 27 and 31 ms. After, it grew by 128 and 112 KiB, and the last
drain took 75 and 90 us.

Each drain row refills every live slot with one sample before the timed drain. The before drain
also allocated a 221 KB histogram per call. About 0.27 ns of the `record_latency_ns` rise came
with the slot registry. The rest comes from the bench input at 2 significant figures: it records
`1_000 + (i & 1023)`, so about 8 consecutive samples increment the same count. With the same values in
scattered order, both precisions read 1.67 to 1.75 ns.

## Field keys

`field::*` exposes OTel-standard dotted attribute keys (`http.request.method`, `rpc.service`,
`error.type`, and so on) for structured log, span, and metric fields. No project-invented
`myapp.*` prefixes.

## License

MIT OR Apache-2.0.
