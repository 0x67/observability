---
lat:
  require-code-mention: true
---
# Tests

Behavioral test specifications for the hot-path gate, sampler, counters, histogram merge, and OTel field keys in `observability-core`.

In `observability` they cover the metrics recorder guard, the OTLP config debug output, the OTLP layer filters, and the init contract.

## Runtime gate

The process-wide gate mirrors into a per-thread `Cell` so the hot path never re-reads the shared atomic once primed.

### Gate defaults off then flips via refresh

Gate starts `false` on a fresh process; calling `set_metrics_enabled(true)` then `refresh_thread_gate` flips the calling thread's cached mirror to match.

### Timer skips clock when gate off, captures when on

Off gate: `timer()` must not read the clock or capture a metric name, no `Instant`, no capture. On gate: `timer()` must capture the start instant and metric name for its drop-time record.

## Sampling

`should_sample` is a wraparound periodic sampler for hot-path call sites that must not fire on every message but must never miss a fresh thread's first call.

### Fires on first call and every period after

`should_sample(mask)` samples true on a thread's very first call regardless of the sampler's internal counter start value, then again exactly once per `mask + 1` consecutive calls, never twice within one period.

## Per-thread message counting

`count_msg`/`take_count` track a per-thread `Cell<u64>`, never a shared atomic, so the hot path never pays a cross-core cache line bounce.

### take_count reads then resets

Reading via `take_count` returns the accumulated count and resets the thread-local counter to zero in the same call; a second read immediately after returns zero.

## Cross-thread histogram and count aggregation

`merge_local` hands a thread's local `hdrhistogram` and message count into shared per-thread registry slots; `drain_all`/`drain_count` fold every registered slot into one snapshot, clearing each slot in place.

### merge_local and drain_all aggregate across threads with clamp

A worker's recorded latencies, plus one sample above the 60s bound, merge via `merge_local`; `drain_all` on another thread sums them with the tail sample clamped into range.

`drain_count` returns the matching message-count sum for the same worker, proving both registries hand off together on one `merge_local` tick.

## OTel-standard field keys

The `field` module ships OTel-standard attribute key constants only; no project-invented prefix is allowed to leak into the exported key set.

### Field keys are dotted OTel names without a project prefix

Every constant in the `field` module equals its documented OTel dotted name (for example `http.request.method`), and none contain a project-prefix marker.

## Metrics recorder guard drop behavior

`MetricsGuard` is the RAII handle for an installed metrics recorder; its `Drop` impl must flip the `observability-core` gate off so a torn-down recorder never leaves the hot path believing metrics are still live.

### Drop flips the gate off

Constructing a `MetricsGuard` flips the `observability-core` gate on; dropping that guard flips it back off, proving the recorder lifecycle and the hot-path gate stay in sync.

## OTLP config debug output

`OtlpConfig` holds header values that are usually credentials, so its hand-written `Debug` must never print them.

### Debug prints header names and hides values

Formatting an `OtlpConfig` that has header `("x-api-key", "s3cr3t-value")` shows the header name paired with `[REDACTED]`, never the value, and still shows endpoint, protocol, resource, and timeout.

## OTLP layer filters

The OTel span layer, the OTLP log bridge, and the bunyan span storage each carry a filter, so the global max level stays at the configured level and exporter threads produce no OTLP telemetry.

### Tracing on keeps global max level and drops debug spans

With tracing on at `level = "info"` against the HTTP mock collector, `LevelFilter::current()` is `INFO`; after a `debug_span!("below")`, an `info_span!("kept")`, and the guard drop, the collector received `kept` and not `below`.

### Log bridge drops exporter thread events

With an OTLP log sink at `info`, a `warn!` with target `tonic::transport` from the test thread reaches the HTTP mock collector, and a `warn!` from a thread named `OpenTelemetry.Logs.BatchProcessor` does not.

### Exporter thread filter drops only exporter thread events

A capture layer behind `otlp_filter("info")` records a `tonic::transport` warning from the test thread and from a thread named `app-worker`, and nothing from the four exporter threads.

The exporter threads are `observability-otlp`, `OpenTelemetry.Traces.BatchProcessor`, `OpenTelemetry.Logs.BatchProcessor`, and `OpenTelemetry.Metrics.PeriodicReader`.

### Bunyan storage layer hint follows most verbose bunyan sink

Sinks bunyan `info`, bunyan `debug`, and json `trace` give the bunyan storage layer a max level hint of `DEBUG`: the json sink's `trace` does not count.

## Init contract

Config the build cannot honor returns a typed error before any subscriber or recorder is installed; each case runs in the feature set that makes it reachable.

### Tracing without otlp returns missing otlp traces

`tracing: Some` with `otlp: None` returns `PipelineError::MissingOtlp(Signal::Traces)` and leaves no global subscriber set.

### OTLP log sink without otlp returns missing otlp logs

An OTLP log sink with `otlp: None` returns `PipelineError::MissingOtlp(Signal::Logs)` and leaves no global subscriber set.

### Tracing without otel feature returns feature disabled

Without the `otel` feature, `tracing: Some` returns `PipelineError::FeatureDisabled { feature: "otel" }` and leaves no global subscriber set.

### OTLP log sink without otel-logs returns feature disabled

Without the `otel-logs` feature, an OTLP log sink returns `PipelineError::FeatureDisabled { feature: "otel-logs" }` and leaves no global subscriber set.

### Bunyan sink without feature returns feature disabled

Without the `bunyan` feature, a bunyan stdout sink returns `PipelineError::FeatureDisabled { feature: "bunyan" }` instead of falling back to json, and leaves no global subscriber set.

### Invalid endpoint returns exporter error

Tracing over HTTP to `http://exa mple:4318` returns `PipelineError::Exporter` with `Signal::Traces` and an `OtlpError::Build` source, and leaves no global subscriber set.

### OTLP metrics with invalid endpoint returns exporter error

`init_metrics` with the OTLP exporter over gRPC to `http://exa mple:4317` returns `MetricsError::Exporter(OtlpError::Build(_))` and leaves the metrics gate off.
