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

```rust
pub fn set_metrics_enabled(on: bool);   // flip the process-wide gate (Relaxed; advisory)
pub fn metrics_enabled() -> bool;       // hot-path check: one Relaxed load
```

`observability::init_metrics` turns the gate on after a successful recorder install, so a
binary rarely calls `set_metrics_enabled` directly.

## Latency, counts, sampling

```rust
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

```rust
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

```rust
use std::time::Duration;

// inside a tokio runtime, after `observability::init` or another `metrics` recorder install
let flusher = observability_core::spawn_flusher(
    "feed.decode_latency",
    "feed.msgs",
    Duration::from_secs(15), // the scrape or export interval
)?;
// dropping `flusher` stops the task
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

## Field keys

`field::*` exposes OTel-standard dotted attribute keys (`http.request.method`, `rpc.service`,
`error.type`, and so on) for structured log, span, and metric fields. No project-invented
`myapp.*` prefixes.

## License

MIT OR Apache-2.0.
