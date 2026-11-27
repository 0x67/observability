# observability

Two-crate observability lib, reusable by any Rust project. [[crates/observability-core/src/lib.rs]] is the hot-path metrics core; [[crates/observability/src/lib.rs]] wires tracing and metrics backends on top of it.

`observability-core` depends only on `metrics` + `hdrhistogram` (+ optional `tokio` flusher), so a hot-path recv loop can pull it without dragging in `opentelemetry` or `tracing-subscriber`. All backend wiring lives one crate up, in `observability`.

## Two-subsystem split

[[crates/observability/src/lib.rs#init]] brings up two subsystems that share nothing at runtime except one `OtlpConfig`.

[[crates/observability/src/pipeline/mod.rs#init_pipeline]] builds a single `tracing_subscriber::Registry`: a logging fan-out over every configured sink (stdout, rolling file, OTLP-log bridge), each with its own format and level filter, fused with exactly one tracing OTLP span sink. Tracing is never fanned out and never written to disk, by design; only logging fans out.

`init_pipeline` builds every layer in `build_layers` before `try_init`, so a config the build cannot honor returns `Err` with no subscriber set: `FeatureDisabled` for a missing cargo feature, `MissingOtlp` for tracing or an OTLP log sink without `otlp`, and `Exporter` (with the `Signal`) when the export runtime or an exporter fails to build. The crate prints nothing to stderr. At most one sink may be `LogSinkKind::Otlp`: a second one returns `PipelineError::DuplicateOtlpLogSink` before anything is built, because two bridges on one logger provider would export every record twice.

[[crates/observability/src/config.rs#TracingConfig]] has its own `level` for exported spans, falling back to the pipeline `level`, and an optional `sample_ratio`. A ratio makes the tracer sample roots with that probability through `ParentBased(TraceIdRatioBased)`, so a child follows its parent, remote ones included. A NaN ratio or one outside `0.0..=1.0` returns `PipelineError::SampleRatio` before anything is built, where the SDK would clamp it. With no ratio the SDK default sampler stays, and it reads `OTEL_TRACES_SAMPLER`.

A rolling file sink whose directory cannot be created or opened returns `PipelineError::RollingFile` with the directory. `max_files` caps the files with the sink's prefix. `tracing-appender` prunes the oldest when the sink opens and on each rotation, which means a restart enforces the cap too.

Each layer in the registry has a filter. One unfiltered layer would leave the registry without a max level hint, which lifts the global max level to `trace`. Both OTel layers, spans and the OTLP log bridge, use [[crates/observability/src/pipeline/filter.rs#otlp_filter]]: the sink's `EnvFilter` combined with [[crates/observability/src/pipeline/filter.rs#ExporterThreadFilter]], which drops events and spans created on the export runtime's threads and the SDK's batch and periodic threads, so exporting never produces more telemetry for the same exporter. Fmt sinks keep only their level filter, and export errors still show in local logs. Bunyan span storage takes the filter of the most verbose bunyan sink.

`ObsGuard`, [[crates/observability/src/pipeline/guard.rs#PipelineGuard]], and `MetricsGuard` are `#[must_use]`. A statement like `init_pipeline(cfg)?;` drops the guard at once, which stops the log writers and turns the metrics gate off, so the lint makes that mistake a compile-time warning.

[[crates/observability/src/metrics/mod.rs#init_metrics]] installs the `metrics`-facade global recorder (Prometheus pull or OTLP push) and flips the `observability-core` gate on success only; any error path leaves the gate untouched. It runs independently of the pipeline: `init` calls `init_pipeline` first (so a metrics failure is itself logged), and a metrics install failure downgrades to a `tracing::warn!` plus `metrics: None`, never taking down logging or tracing.

The Prometheus recorder takes a [[crates/observability/src/config.rs#PrometheusConfig]]: a listen address (`127.0.0.1:9000` by default), bucketed histograms with the Prometheus client default seconds buckets (or quantile summaries), and an optional idle timeout. Every series gets a `service_name` label with the service name `init_metrics` receives, the Prometheus spelling of `service.name`. The idle timeout drops counter and histogram series not updated for that long and keeps gauges, so a gauge set once stays. The exporter builds with only its `http-listener` feature, which leaves out the push-gateway client and its `aws-lc-rs` provider.

The subsystems are coupled at runtime in two places. One is the `OtlpConfig` value `init` clones out of `PipelineConfig` before calling `init_pipeline`, then hands to `init_metrics` alongside the service name. The other is the export runtime, which both get from `ExportRuntime::shared`.

## Config surface

Every config type has a `Default` that builds a working pipeline with no tokio runtime, so a caller sets only the fields that differ with `..Default::default()`.

[[crates/observability/src/config.rs#PipelineConfig]] defaults to stdout JSON logging at `info` for service `unknown_service`, the SDK's fallback name, with no OTLP and no tracing. [[crates/observability/src/lib.rs#ObsConfig]] adds no metrics recorder. `OtlpConfig` defaults to HTTP/protobuf at `http://localhost:4318` with a 10 s timeout, the OTLP defaults. `MetricsExporter` defaults to Prometheus on `127.0.0.1:9000`, loopback so a default build does not serve metrics on every interface. OTLP headers and resource attributes are `BTreeMap`s, one entry per key, iterated in key order.

The `serde` feature derives `Deserialize` for every config type, TLS settings included, so a service reads its config from a TOML, YAML, or JSON file. Names are snake_case, an unknown field fails (struct variants of `LogSinkKind` and `MetricsExporter` too), omitted fields come from the type's `Default`, and a `Duration` is integer milliseconds in a field ending in `_ms`, such as `timeout_ms`. Enums use serde's external tagging: `kind = "stdout"`, or `kind = { rolling_file = { ... } }`. There is no `Serialize`, because it would write header values, usually credentials, back out.

## Hot-path memory model (observability-core)

`observability-core` writes only thread-local state on the hot path; its one shared read is the gate flag. Everything sampled per-message lives in thread-local state, handed to one shared slot per thread only on a periodic tick or at thread exit.

The runtime gate is one private process-wide `AtomicBool`. [[crates/observability-core/src/gate.rs#set_metrics_enabled]] writes it and [[crates/observability-core/src/gate.rs#metrics_enabled]] reads it with a `Relaxed` load on every call, with no per-thread copy. The flag is written rarely and stays in each core's cache, so the load costs the same as a thread-local read. A write is visible on every thread without a refresh call.

Latency samples land in a thread-local bounded `hdrhistogram::Histogram` (1ns..60s, 2 sig figs so each histogram is 30 KB with buckets about 1% wide, `saturating_record` so a tail spike clamps into range instead of panicking) via [[crates/observability-core/src/handoff.rs#record_latency_ns]]. The histogram is allocated on a thread's first sample, so a thread that only merges never allocates one. [[crates/observability-core/src/handoff.rs#merge_local]] hands that thread's samples, plus its message count, to the thread's one shared slot, registering the slot on the first hand-off with data; a merge with nothing pending doesn't touch shared state. When the thread exits, its thread-local destructor hands off whatever it has not merged and marks the slot retired. [[crates/observability-core/src/handoff.rs#drain_into]] resets a caller-owned [[crates/observability-core/src/handoff.rs#LatencyWindow]], adds every slot's histogram and count into it, and clears each slot in place. It removes a slot once it has read the retired flag before draining that slot, so thread churn does not grow memory and a flusher only ever sees data a worker has already handed off.

Under `flush-tokio`, [[crates/observability-core/src/flush.rs#spawn_flusher]] runs one task that drains into one reused window once per caller-chosen interval. Each tick adds the drained message count to a counter and sets `.p50`, `.p99`, `.p999`, and `.max` gauges in seconds plus a `.samples` gauge; an empty window updates only `.samples`, so idle time never reads as 0 s. The metric handles are built once at spawn, so the call must follow the recorder install. It returns a [[crates/observability-core/src/flush.rs#FlusherError]] outside a runtime, for a zero interval, or while another flusher runs, and dropping the returned [[crates/observability-core/src/flush.rs#Flusher]] aborts the task.

Message counting mirrors the same shape: a plain per-thread `Cell<u64>` on the hot path (`count_msg`/`take_count`), never a shared atomic, handed to the same slot as the histogram on the same `merge_local` tick or at thread exit. The first `count_msg` after a reset also touches the thread-local state once, so a thread that only counts still hands its count off at exit.

[[crates/observability-core/src/sample.rs#should_sample]] is a per-thread wraparound sampler (`mask` must be `2^n - 1`, e.g. `SAMPLE_1_IN_8192`) for hot-path call sites that need periodic-not-every-message sampling. It samples true on a fresh thread's very first call, then exactly once per `mask + 1` consecutive calls after that.

[[crates/observability-core/benches/hot_path.rs]] measures these calls with std and `hdrhistogram` only. It prints batch-mean nanoseconds per call (p50, p99, p999, max) for each hot-path call and for drains over 1, 8, and 64 live slots, then the RSS growth and drain time after 2,000 short-lived recording threads. Run it with `cargo bench -p observability-core --bench hot_path`.

## Shared OTLP, three sibling exporters

[[crates/observability/src/otlp/mod.rs#OtlpConfig]] is the single source of endpoint, protocol, headers, resource attrs, timeout, and TLS for every OTLP signal.

`endpoint` is the collector base URL. HTTP export drops one trailing `/` and appends `/v1/traces`, `/v1/logs`, or `/v1/metrics`, because opentelemetry-otlp sends to a programmatic HTTP endpoint as given. gRPC uses the endpoint unchanged, since the service path is part of each gRPC method.

Its `Debug` output prints each header name with `[REDACTED]` in place of the value, because header values usually carry API keys. `PipelineConfig` derives `Debug` and embeds the `OtlpConfig`, so `?cfg` in a log line is safe for both.

An endpoint with a user name or password (an `@` before the first `/` after the scheme) returns `OtlpError::EndpointCredentials` as the first step of every exporter setup. Later errors, opentelemetry-otlp's `InvalidUri` among them, print the endpoint, and this check runs before any of them. Its own message leaves the endpoint out and tells the user to pass credentials in `headers`.

The second step requires an `http://` or `https://` prefix, compared without case, and returns `OtlpError::EndpointScheme` with the endpoint otherwise. opentelemetry-otlp can treat a scheme-less gRPC endpoint as `https://` with no trusted roots, while `is_https` would skip the crate's TLS setup for it. Both setups then pin the upstream protocol in code (`Protocol::Grpc` or `Protocol::HttpBinary`), so `OTEL_EXPORTER_OTLP_PROTOCOL` and its per-signal variables cannot change the transport.

`pipeline/otel.rs` and `metrics.rs` each call its exporter builders directly: `span_exporter` for tracing, `metric_exporter` (under `metrics-otel`) for the metrics bridge, `log_exporter` (under `otel-logs`) for the log bridge layer, all off the same endpoint/timeout/headers pair. No `opentelemetry` type crosses into a public `observability` signature; the builders stay `pub(crate)` and otel-gated.

[[crates/observability/src/otlp/runtime.rs#ExportRuntime]] is the one tokio runtime (multi-thread, 1 worker, threads named `observability-otlp`) that runs every exporter's I/O, so OTLP export works with or without an app runtime. gRPC exporters are built inside its context, so tonic spawns its channel worker there. HTTP exporters send through [[crates/observability/src/otlp/http_client.rs#RuntimeHttpClient]], an async reqwest client whose requests are spawned onto that runtime.

Dropping a guard shuts its OTel providers down, which exports the final batch, and logs a failed shutdown as a warning. Export runs on the export runtime, so this works from a thread with no runtime and from inside a current_thread or multi-thread app runtime, async context included. `ObsGuard` declares `metrics` before `pipeline`. The metrics recorder therefore shuts down first, while the log sinks can still print its warnings.

`ExportRuntime::shared` hands out the live runtime through a `Weak` slot, so the pipeline and metrics guards share one export thread, and an init after every guard dropped starts a fresh one. Each guard declares its `Arc` after its providers, which keeps the runtime running through their final flush. The last `Arc` to drop calls `shutdown_background`, which is safe inside an async context.

Every configured header goes out on both transports: as HTTP headers, or as gRPC metadata through `MetadataMap::from_headers`. A header whose name or value is not valid HTTP returns `OtlpError::InvalidHeader` at init, naming the header and never its value, because opentelemetry-otlp would drop it without a word.

TLS applies only to `https://` endpoints, so `http://localhost` stays plain text. [[crates/observability/src/otlp/tls.rs#OtlpTls]] holds the TLS settings, and the default `tls` feature turns TLS on. `OtlpTls::roots` selects the OS store (`Native`, checked at init for at least one certificate so both transports fail the same way when it is empty), the bundled Mozilla set (`Webpki`, under `tls-webpki-roots`), only `ca_file` (`CaFileOnly`), or the OS verifier through `rustls-platform-verifier` (`Platform`, under `tls-platform-verifier`, for desktop and mobile apps). HTTP passes a finished rustls config to reqwest's `use_preconfigured_tls`. gRPC builds its own tonic `Channel` with `Endpoint::tls_config_with_verifier` and hands it to opentelemetry-otlp's `with_channel`. `ca_file` adds its CA certificates to any roots choice, and `client_cert` presents a certificate for mutual TLS. Both files are read and parsed at init, so a missing or malformed file returns `OtlpError::TlsFile` or `OtlpError::InvalidPem` with its path on either transport. Without `tls`, an `https://` endpoint returns `OtlpError::FeatureDisabled`. ring is the crypto provider unless the app installs a process-wide one first.

`otel_resource` folds `service.name` plus every configured `resource` attribute onto whichever provider calls it, so span, metric, and log all carry identical resource attributes from one config, with no per-signal drift.

## Redaction

The crate redacts only its own config: `OtlpConfig`'s `Debug` hides header values, and init rejects an endpoint with credentials. Application values are left to secret types in the app and to an OTel Collector.

The `## Redaction` section of [[crates/observability/README.md]] covers `secrecy::SecretString` and `veil::Redact`, the collector config in `docs/otel-collector-redaction.yaml`, and why an `http://` endpoint suits only a collector on the same host. It also lists what the crate never redacts: log messages, OTLP log bodies, span names, span event names, metric labels, and resource attributes.

## Bounded delivery and drop counters

Every queue between the app and a sink has a fixed bound, and every item lost to a full queue is counted.

Each stdout and rolling-file sink writes through [[crates/observability/src/pipeline/writer.rs#sink_writer]], a `tracing-appender` `NonBlocking` sized by the sink's `LineQueue`: 128,000 lines and `Overflow::Drop` by default, the `tracing-appender` defaults. `Overflow::Block` makes the logging thread wait for space instead, which loses nothing and stalls that thread on a slow disk.

A lossy `NonBlocking` returns `Ok` for a dropped line and only bumps its `ErrorCounter`, and it has no callback. [[crates/observability/src/pipeline/writer.rs#CountingWriter]] therefore reads that counter after each write and reports any growth, so the write that lost a line reports it. The fast path is two relaxed loads per line.

[[crates/observability/src/pipeline/dropped.rs#DropCounters]] holds one pipeline's totals per signal. `add` updates the total and increments the `observability.dropped` counter with `signal` and `reason` labels, which Prometheus writes as `observability_dropped`. A drop before a metrics recorder is installed reaches only the total. `PipelineGuard::dropped` returns the totals as a [[crates/observability/src/pipeline/dropped.rs#Dropped]], so a service without a recorder can read them too.

Span export goes through [[crates/observability/src/pipeline/bounded/traces.rs#BoundedSpanProcessor]] in front of the SDK's `BatchSpanProcessor`, whose exporter is wrapped by a counter. The bound is `TracingConfig::batch.queue_size`, an [[crates/observability/src/config.rs#ExportBatch]] set explicitly, so `OTEL_BSP_*` is not read: the SDK keeps its resolved queue size private, and exact counting needs it.

The SDK drops on a full channel with only a log line, so the bounded processor decides first. A span takes a slot in a shared `InFlight` count when it ends and frees it when its batch's export returns. That count never falls below the spans in the SDK channel of the same capacity, so the SDK never finds the channel full and every drop is counted here, as `queue_full`. A failed export counts its batch as `export_failed` and still frees its slots. Spans in the worker's pending batch and in a running export count against the bound too, so memory stays under `queue_size` spans in total. Both wrappers forward `set_resource`, the only way the provider's resource reaches the OTLP exporter.

The OTLP log sink has the same chain, [[crates/observability/src/pipeline/bounded/logs.rs#BoundedLogProcessor]] in front of `BatchLogProcessor`, bounded by `LoggingConfig::otlp_batch`, whose default delay is 1 s, the SDK log default. `OTEL_BLRP_*` is not read either. The log exporter counts a batch with `batch.iter().count()`, since `LogBatch` has no `len()`.

## Trace context propagation

With tracing on, a successful [[crates/observability/src/pipeline/mod.rs#init_pipeline]] sets the W3C `TraceContextPropagator` as the process-wide OTel text map propagator, so OTel-aware HTTP and gRPC middleware works without setup.

It replaces a propagator set before `init`, and one set after `init` wins. A failed init leaves the global alone.

[[crates/observability/src/propagation.rs#trace_headers]] and [[crates/observability/src/propagation.rs#set_parent_from_headers]] carry `traceparent` and `tracestate` as plain string pairs, so no `opentelemetry` type reaches the public API. Header names match without regard to case. Without a valid `traceparent`, `set_parent_from_headers` returns `Ok` and leaves the span's local parent in place, because handing `set_parent` an empty context would make the span a root. Misuse maps `tracing-opentelemetry`'s `SetParentError` one to one onto [[crates/observability/src/error.rs#PropagationError]]: no OTel layer, span already started, span filtered out.

A span's OTel context starts when it is first entered or its headers are read, and its parent is fixed from then on, so callers set the remote parent first.

## Feature model

`otel` is the grouping feature. Per-signal features layer on top of it.

`otel` pulls in `opentelemetry`/`opentelemetry_sdk`/`opentelemetry-otlp`/`tracing-opentelemetry`, plus `tokio` and `reqwest` for the export runtime, and gates every `#[cfg(feature = "otel")]` block. `tls` (default) adds rustls with ring for tonic and reqwest and the OS store check; `tls-webpki-roots` adds the bundled Mozilla roots; `tls-platform-verifier` adds `rustls-platform-verifier`. `otel-traces` is a plain alias for `otel`. `otel-logs` adds the OTLP log bridge (`opentelemetry-appender-tracing`). `metrics-prom` adds the Prometheus pull exporter and `metrics-util` for its `MetricKindMask`. `metrics-otel` adds the OTLP metrics bridge and implies `otel`. `bunyan` adds the bunyan-format log sink. `serde` adds `Deserialize` to the config types.

No feature is named `tracing` or `metrics`; those names stay reserved for the extern facade crate deps themselves, so a feature flag never shadows `tracing::` or `metrics::` path resolution.

The whole otel stack pins to opentelemetry 0.31: `metrics-exporter-otel` 0.3.1 pins `opentelemetry ^0.31`, and all three OTLP signal exporters plus the metrics bridge must resolve to one `opentelemetry` version, so that crate is the version bottleneck for the entire stack.

## See also

Related sections outside this file:

- [[tests]] documents test specifications for the runtime gate, sampler, per-thread counters, and histogram merge.
