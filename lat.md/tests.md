---
lat:
  require-code-mention: true
---
# Tests

Behavioral test specifications for the hot-path gate, sampler, counters, histogram merge, and slot retirement in `observability-core`.

In `observability` they cover the config defaults and file format, tracing level and sampling, trace context propagation, the writer sink queue, drop counters, and bounded export queues, the Prometheus recorder, the OTLP metrics recorder, the metrics recorder guard, the OTLP config debug output, OTLP endpoint credentials and scheme, the OTLP layer filters, the init contract, the rolling file sink, OTLP headers, OTLP signal paths, TLS export, and guard drop.

## Runtime gate

The process-wide gate is one atomic that every thread reads directly, with no per-thread copy to refresh. Gate tests hold the crate's `TEST_STATE` lock, because the gate is shared by every test in the binary.

### Toggle reaches other threads without refresh

With the gate off, a spawned thread reads `metrics_enabled() == false` and waits. The main thread calls `set_metrics_enabled(true)`, and the spawned thread's next read returns `true` with no other call in between.

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

`merge_local` hands a thread's local `hdrhistogram` and message count to its shared slot; `drain_into` resets a caller-owned window, folds every slot into it, and returns the message count.

### merge_local and drain_into aggregate across threads with clamp

A worker's recorded latencies, plus one sample above the 60s bound, merge via `merge_local`; `drain_into` on another thread sums them with the tail sample clamped into range.

The same drain returns the worker's message count, proving latency and count hand off together on one `merge_local` tick.

### Window has two sigfig bounds

A default `LatencyWindow` covers 1 ns to 60 s at 2 significant figures, which hdrhistogram stores in 3,840 counts (30,720 bytes). The test pins that footprint, the same for every per-thread and slot histogram.

### Drain resets the window before filling

Ten 1,000 ns samples drain into a window. Three 2,000 ns samples then drain into the same window, which afterwards holds exactly those three and no 1,000 ns sample.

## Slot retirement

Each recording thread owns one shared slot; its exit hands off unmerged data and retires the slot, and the next drain removes it. These tests hold `TEST_STATE`, because the slot registry is process-wide.

### Exited threads release their slots

50 threads each record 1,000 ns once and merge, then exit. Their 50 slots stay registered until the next drain, which reads all 50 samples and leaves no slot behind.

### Live thread keeps its slot

A thread that records, merges, and stays alive keeps its slot across a drain. After it exits, the next drain removes the slot.

### Exit hands off unmerged samples

20 threads each record three samples and count five messages without ever merging, then exit. The next drain returns all 60 samples and 100 messages and leaves no slot behind.

### Count-only thread hands off at exit

A thread counts three messages and exits without recording or merging. The next drain returns a message count of 3 and leaves no slot behind.

### Idle merge registers no slot

A thread that never records or counts calls `merge_local` three times. No slot is registered while it runs or when it exits.

### Count-only merge registers a slot

A thread that counts one message and merges registers exactly one slot. The next drain returns a message count of 1 and an empty window.

### Record after teardown does not panic

A thread-local destructor that runs after the core's thread-local state is gone calls `record_latency_ns`, `count_msg`, and `merge_local`. Each call drops its data, and the thread joins without a panic.

## Tokio flusher

Flusher tests run a current-thread runtime with paused time and read metrics from a `DebuggingRecorder` set with `metrics::with_local_recorder`. They hold `TEST_STATE`, because the slots and the running flag are process-wide.

### Publishes window quantiles in seconds

A worker records 100 samples of 1 ms, one of 50 ms, and 7 messages, merges, and exits. One 10 s window later, the gauges report those values in seconds.

`lat.p50` is within 1% of 0.001, `lat.max` within 1% of 0.05, `lat.samples` is 101, and `msgs` is 7. `lat.p99` is described with `Unit::Seconds` and `lat.samples` with `Unit::Count`. A second window of ten 2 ms samples moves `lat.p50` to within 1% of 0.002 through the same handles.

### Empty window keeps last quantiles

After the first window above, a window with no samples sets `lat.samples` to 0 and leaves `lat.p50` within 1% of 0.001.

### Spawn outside a runtime returns NoRuntime

With no runtime, `spawn_flusher` returns `FlusherError::NoRuntime`. A following call inside a runtime succeeds, which shows the failed call left the running flag clear.

### Second spawn while running is refused

While the first `Flusher` is alive, a second `spawn_flusher` returns `FlusherError::AlreadyRunning`.

### Dropping the handle allows a respawn

After the `Flusher` is dropped, a worker merges one sample and one message, and 30 s of paused time pass without the `msgs` counter moving. A new `spawn_flusher` then succeeds.

### Zero interval is refused

`spawn_flusher` with `Duration::ZERO` returns `FlusherError::ZeroInterval` instead of letting tokio panic.

## Hot-path allocation

`tests/hot_path_alloc.rs` installs a global allocator that counts each thread's `alloc` and `realloc` calls, so any allocation on the per-message path fails the build's tests.

### Steady-state hot path does not allocate

After one warm-up record, merge, and drain, 10,000 rounds of `metrics_enabled`, `record_latency_ns`, `count_msg`, `take_count`, `should_sample`, `merge_local`, and `drain_into` into one reused window don't call the allocator once.

## Config defaults

The default config values build a working pipeline, so a caller sets only the fields that differ.

### Default ObsConfig inits without a runtime

`init(ObsConfig::default())` on a plain test thread with no tokio runtime returns `Ok`, and the guard holds no metrics recorder.

### Default OtlpConfig exports HTTP protobuf to the base URL

`OtlpConfig { endpoint: <mock base URL>, ..Default::default() }` with `TracingConfig::default()` and no app runtime sends the span in a `POST /v1/traces` with `content-type: application/x-protobuf`.

A gRPC default would send nothing the HTTP mock accepts.

## Tracing config

`TracingConfig` filters exported spans by its own level and samples root spans by its ratio.

### Sample ratio outside unit range is rejected

`build_layers` with a sample ratio of `-0.1`, `1.5`, or NaN returns `PipelineError::SampleRatio` with that ratio. Ratios `0.0` and `1.0` pass the check and fail on the missing `otlp` instead.

### Tracing level filters exported spans

With pipeline level `info`, a stdout sink at `info`, and `TracingConfig::level` `warn`, the HTTP mock receives `warn_span!("warn-span")` and not `info_span!("info-span")`.

### Zero ratio exports no root spans

With `sample_ratio` 0 the HTTP mock never receives the root span `root-span`. A span whose parent is a sampled remote span context arrives, so spans do reach the mock under this sampler.

## Writer sink queue

A stdout or rolling-file sink buffers at most its `LineQueue::lines`. These tests write 10 lines into a one-line queue whose output blocks until the test opens a gate.

### Drop overflow counts lost lines

With `Overflow::Drop`, the delivered lines plus `Dropped::logs` come to exactly 10 once the gate opens and the worker guard drops, and at least 8 lines are counted as lost.

### Block overflow delivers every line

With `Overflow::Block`, a second thread writes the 10 lines and waits on the full queue until the gate opens. All 10 arrive in order, and `Dropped::logs` stays 0.

## Drop counters

`DropCounters` keeps one pipeline's per-signal totals and mirrors each drop into the `observability.dropped` counter.

### Add updates totals and metric

Under a `DebuggingRecorder` set with `metrics::with_local_recorder`, 2 spans dropped as `QueueFull` and 3 log lines as `BufferFull` give `Dropped { traces: 2, logs: 3 }`.

`observability.dropped` reads 2 for `signal="traces"`, `reason="queue_full"` and 3 for `signal="logs"`, `reason="buffer_full"`.

## Bounded export queues

The bounded span chain runs against test exporters whose first export waits on a gate, fails, or succeeds.

### Bounded span processor drops and counts past queue size

With `queue_size` 4, `max_batch_size` 1, and the first export stalled, 10 ended spans leave `Dropped::traces` at 6. After the gate opens and the provider shuts down, the exporter has received exactly 4 spans and the count is still 6.

### Failed export frees slots and counts the batch

With `queue_size` 4 and a first export that fails, 3 spans and a flush give `Dropped::traces` 3. Four more spans then all fit, and the next flush exports all 4.

### Bounded log processor drops and counts past queue size

The log chain with `queue_size` 4, `max_batch_size` 1, and the first export stalled: 10 emitted records leave `Dropped::logs` at 6, and the exporter has received exactly 4 records after shutdown.

### Wrappers forward resource to exporter

A provider with resource `service.name=res-probe` passes it through both wrappers: after a flush, the test exporter's last `set_resource` call holds `service.name` `res-probe`.

## Prometheus recorder

A Prometheus scrape shows the configured histogram format, labels, and idle expiry.

### Scrape shows buckets service label and expiry

`init_metrics` on a free port with a 300 ms idle timeout and service `scrape-test` records a histogram, a counter, and a gauge.

The first scrape has a `req_seconds_bucket` line with `le="0.25"`, the counter, and `service_name="scrape-test"` on every series line.

One second later the second scrape has no `hits` counter and still has the `limit` gauge. The wait is real time, because the exporter's idle check reads the wall clock.

## Trace context propagation

The string API carries W3C trace context in and out of a span without any `opentelemetry` type. The tests share one pipeline per process with `sample_ratio` 0.

### Remote parent round trips and overrides zero ratio

Header `TraceParent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01` (mixed case) on a new span, then `trace_headers`: `set_parent_from_headers` returns `Ok`.

The written `traceparent` keeps the remote trace id, has a new span id, and ends in `-01` although the ratio is 0, because the sampler follows a sampled parent.

### Set parent after headers returns already started

`trace_headers` on a span starts its OTel context, so a following `set_parent_from_headers` returns `PropagationError::AlreadyStarted`.

### Headers without traceparent keep the local parent

A child created inside a local span, given headers with no `traceparent`, gets `Ok` and keeps the local parent's trace id.

### Set parent without tracing returns no OTel layer

In a logging-only pipeline, `set_parent_from_headers` with a valid `traceparent` returns `PropagationError::NoOtelLayer`.


## Config files

With the `serde` feature, the whole config reads from a file. These tests parse JSON; TOML and YAML use the same names.

### Config file parses into expected values

A JSON document with a gRPC `otlp` table (`timeout_ms` 2500, one header, one resource attribute) and a stdout plus a rolling-file sink parses into those values.

Omitted fields come from each type's `Default`: level `info`, `Rotation::Daily`, and no metrics. The rolling sink keeps `max_files` 3 and `LogFormat::Pretty`.

### TLS settings parse from file

An `otlp.tls` table with `roots = "webpki"`, a `ca_file`, and a `client_cert` gives `TlsRoots::Webpki` and both paths. A `client_cert` without `key_file` fails and names the field.

### Logging otlp batch defaults to one second delay when omitted

`{"pipeline":{"logging":{"sinks":[]}}}` gives `otlp_batch.delay` of 1 s, `LoggingConfig`'s default, not the 5 s of `ExportBatch::default()`.

### Prometheus settings parse from file

A `metrics.exporter.prometheus` table with `bind`, `histograms = "summaries"`, and `idle_timeout_ms` 300000 parses into the matching `PrometheusConfig`, the idle timeout as 300 s.

### Unknown field is rejected

A misspelled `servce_name`, and a misspelled `max_file` inside a `rolling_file` sink, each fail with an error that names the unknown field.

## Metrics recorder guard drop behavior

`MetricsGuard` is the RAII handle for an installed metrics recorder; its `Drop` impl must flip the `observability-core` gate off so a torn-down recorder never leaves the hot path believing metrics are still live.

### Drop flips the gate off

Constructing a `MetricsGuard` flips the `observability-core` gate on; dropping that guard flips it back off, proving the recorder lifecycle and the hot-path gate stay in sync.

## OTLP metrics recorder

[[crates/observability/src/metrics/otel_recorder/mod.rs#OtelRecorder]] is checked through `metrics::with_local_recorder` on an `SdkMeterProvider` over the SDK's in-memory exporter, read after one `force_flush`. No test installs a global recorder.

### Counter labels become attributes

`counter!("feed.msgs", "venue" => "x")` incremented by 3 and the same name with `"venue" => "y"` incremented by 5 export as one monotonic `u64` sum named `feed.msgs` with two points: 3 with `venue=x` and 5 with `venue=y`.

### Gauge reports last value after set and increment

`set(2.5)` then `increment(0.5)` on one gauge exports a single `f64` gauge point of 3.0, which fails if the stored f64 bits are written or read as an integer.

### Histogram reports count and sum

`record(0.25)` and `record(0.75)` on a histogram labelled `venue=x` export one `f64` histogram point with count 2, sum 1.0, and the label as its only attribute.

### Described gauge exports UCUM unit and text

`describe_gauge!` with `Unit::Seconds` and `"p99 window"` before the gauge's first use exports unit `s` and that description, through the observable instrument builder.

### Described histogram exports UCUM unit and text

`describe_histogram!` with `Unit::Bytes` and `"payload size"` before the histogram's first use exports unit `By` and that description, through the histogram builder.

## OTLP config debug output

`OtlpConfig` holds header values that are usually credentials, so its hand-written `Debug` must never print them.

### Debug prints header names and hides values

Formatting an `OtlpConfig` that has header `("x-api-key", "s3cr3t-value")` shows the header name paired with `[REDACTED]`, never the value, and still shows endpoint, protocol, resource, and timeout.

## OTLP endpoint credentials

A user name or password in the endpoint would reach error text, upstream URI errors included, so exporter setup rejects it before any other step.

### Endpoint credentials are rejected before other checks

`https://user:pa55word@collector:4318` over HTTP, `http://user:pa55word@collector:4317` and `http://token@collector:4317` over gRPC, and the scheme-less `user:pa55word@collector:4318` over HTTP each return `OtlpError::EndpointCredentials`.

Each config also has a header that the next check rejects, so only a credential check that runs first passes. Neither the error's `Display` nor its `Debug` contains the password or the host.

### At sign outside authority is not a credential

An `@` in the path or query (`http://collector:4318/v1/traces?x=a@b`, `http://collector:4318/users/a@b/traces`) is not a credential, and neither is an endpoint without one (`collector:4317`).

## OTLP endpoint scheme

Exporter setup accepts only `http://` and `https://` endpoints and pins the transport to `OtlpConfig::protocol`, whatever the OTLP protocol variables say.

### Endpoint without scheme is rejected on both protocols

`collector.example:4317` and `unix:///tmp/otel.sock` over gRPC and `collector.example:4318` over HTTP return `OtlpError::EndpointScheme` with that endpoint in its `endpoint` field.

`HTTP://127.0.0.1:4318` over HTTP builds, because the scheme is compared without case.

### Exporter follows configured protocol over env

An HTTP exporter builds with `OTEL_EXPORTER_OTLP_PROTOCOL=grpc` or `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=grpc` set, and a gRPC exporter builds with `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf` set. Each variable is restored after its build.

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

### Two OTLP log sinks return duplicate error

Two `LogSinkKind::Otlp` sinks return `PipelineError::DuplicateOtlpLogSink` and leave no global subscriber set. `otlp` is unset, so the check runs before the provider setup, which would fail on it first.

### Bunyan sink without feature returns feature disabled

Without the `bunyan` feature, a bunyan stdout sink returns `PipelineError::FeatureDisabled { feature: "bunyan" }` instead of falling back to json, and leaves no global subscriber set.

### Invalid endpoint returns exporter error

Tracing over HTTP to `http://exa mple:4318` returns `PipelineError::Exporter` with `Signal::Traces` and an `OtlpError::Build` source, and leaves no global subscriber set.

### OTLP metrics with invalid endpoint returns exporter error

`init_metrics` with the OTLP exporter over gRPC to `http://exa mple:4317` returns `MetricsError::Exporter(OtlpError::Build(_))` and leaves the metrics gate off.

### Endpoint with credentials returns exporter error without the secret

Tracing over HTTP to `http://user:s3cret@127.0.0.1:1` returns `PipelineError::Exporter` with `Signal::Traces` and an `OtlpError::EndpointCredentials` source. Neither the error's `Debug` nor any `Display` in its source chain contains `s3cret`.

## Rolling file sink

A rolling file sink fails init with a typed error instead of a panic, and keeps at most `max_files` files with its prefix.

### Rolling appender prunes to max files

A temp dir holding `app.2020-01-01` to `app.2020-01-04`, opened as a daily `app` sink with `max_files = 2`, holds at most 2 files with prefix `app` once the appender is built.

### Rolling file in unwritable dir returns err

A rolling sink whose `dir` sits below a regular file returns `PipelineError::RollingFile` with that `dir`, without a panic and without setting a subscriber.

## OTLP headers

Configured headers reach the collector on both transports, and an invalid header fails init without leaking its value.

### gRPC export without app runtime sends headers as metadata

From a plain test thread with no tokio runtime, headers `x-api-key: k1` and `X-Tenant: t1` over gRPC reach the gRPC mock collector as metadata `x-api-key: k1` and `x-tenant: t1`, along with the span.

### HTTP export sends configured headers

From a test thread with no tokio runtime, header `x-api-key: k1` over HTTP reaches the HTTP mock collector with the span.

### Invalid header is rejected on both protocols

A header named `x api`, or `x-api-key` with value `k1` plus a newline, returns `OtlpError::InvalidHeader` with that name on gRPC and HTTP, and the error text omits the value.

## OTLP signal paths

`OtlpConfig::endpoint` is the collector base URL, and HTTP export appends each signal's path to it.

### Signal URL appends the signal path

`http://c:4318` with `/v1/traces` gives `http://c:4318/v1/traces`. `http://c:4318/` with `/v1/logs` gives `http://c:4318/v1/logs`, with one slash. `https://c/otlp` with `/v1/metrics` gives `https://c/otlp/v1/metrics`.

### One endpoint sends each signal to its path

One HTTP/protobuf `OtlpConfig` at the HTTP mock's base URL drives tracing, an OTLP log sink, and OTLP metrics through `init`.

After one span, one `info!`, one counter, and the guard drop, the span arrived in a `POST /v1/traces`, the log record in a `POST /v1/logs`, and the counter in a `POST /v1/metrics`. Every traces request carries the configured `service.name`.

## TLS export

Export over TLS to the mock collectors, which serve certificates from a PKI that `rcgen` generates at test time.

### Custom CA and client cert export on both transports

With `TlsRoots::CaFileOnly`, the test CA as `ca_file`, and the test client certificate, traces over gRPC and metrics over HTTP reach TLS mocks that require a client certificate from the test CA.

### Platform verifier with extra root exports on both transports

With `TlsRoots::Platform`, the test CA as `ca_file` (extra roots for the OS verifier), and the test client certificate, traces over gRPC and metrics over HTTP reach the TLS mocks.

### Export without client cert is rejected

The same mocks receive nothing from a config without `client_cert`, on either transport, and dropping both guards still returns.

## Guard drop

Dropping `ObsGuard` inside an app runtime delivers the final span and metric without a panic, whatever the runtime flavor.

### ObsGuard drop inside current_thread runtime delivers final batch

Inside a current_thread runtime, `init` with gRPC tracing and gRPC metrics, a final span and counter, then dropping `ObsGuard` in the async block: the drop returns within 2 s, and the gRPC mock received the span and the counter.

### ObsGuard drop inside async context delivers over HTTP

The same steps on a multi-thread runtime over HTTP: the drop inside the async block does not panic, and the HTTP mock received the span and the counter.
