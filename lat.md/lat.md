# observability

Two-crate observability lib, reusable by any Rust project. [[crates/observability-core/src/lib.rs]] is the hot-path metrics core; [[crates/observability/src/lib.rs]] wires tracing and metrics backends on top of it.

`observability-core` depends only on `metrics` + `hdrhistogram` (+ optional `tokio` flusher), so a hot-path recv loop can pull it without dragging in `opentelemetry` or `tracing-subscriber`. All backend wiring lives one crate up, in `observability`.

## Two-subsystem split

[[crates/observability/src/lib.rs#init]] brings up two subsystems that share nothing at runtime except one `OtlpConfig`.

[[crates/observability/src/pipeline/mod.rs#init_pipeline]] builds a single `tracing_subscriber::Registry`: a logging fan-out over every configured sink (stdout, rolling file, OTLP-log bridge), each with its own format and level filter, fused with exactly one tracing OTLP span sink. Tracing is never fanned out and never written to disk, by design; only logging fans out.

`init_pipeline` builds every layer in `build_layers` before `try_init`, so a config the build cannot honor returns `Err` with no subscriber set: `FeatureDisabled` for a missing cargo feature, `MissingOtlp` for tracing or an OTLP log sink without `otlp`, and `Exporter` (with the `Signal`) when the export runtime or an exporter fails to build. The crate prints nothing to stderr. One logger provider serves every OTLP log sink, and each sink gets its own bridge layer at its own level.

A rolling file sink whose directory cannot be created or opened returns `PipelineError::RollingFile` with the directory. `max_files` caps the files with the sink's prefix. `tracing-appender` prunes the oldest when the sink opens and on each rotation, which means a restart enforces the cap too.

Each layer in the registry has a filter. One unfiltered layer would leave the registry without a max level hint, which lifts the global max level to `trace`. Both OTel layers, spans and the OTLP log bridge, use [[crates/observability/src/pipeline/filter.rs#otlp_filter]]: the sink's `EnvFilter` combined with [[crates/observability/src/pipeline/filter.rs#ExporterThreadFilter]], which drops events and spans created on the export runtime's threads and the SDK's batch and periodic threads, so exporting never produces more telemetry for the same exporter. Fmt sinks keep only their level filter, and export errors still show in local logs. Bunyan span storage takes the filter of the most verbose bunyan sink.

`ObsGuard`, [[crates/observability/src/pipeline/guard.rs#PipelineGuard]], and `MetricsGuard` are `#[must_use]`. A statement like `init_pipeline(cfg)?;` drops the guard at once, which stops the log writers and turns the metrics gate off, so the lint makes that mistake a compile-time warning.

[[crates/observability/src/metrics.rs#init_metrics]] installs the `metrics`-facade global recorder (Prometheus pull or OTLP push) and flips the `observability-core` gate on success only; any error path leaves the gate untouched. It runs independently of the pipeline: `init` calls `init_pipeline` first (so a metrics failure is itself logged), and a metrics install failure downgrades to a `tracing::warn!` plus `metrics: None`, never taking down logging or tracing.

The subsystems are coupled at runtime in two places. One is the `OtlpConfig` value `init` clones out of `PipelineConfig` before calling `init_pipeline`, then hands to `init_metrics` alongside the service name. The other is the export runtime, which both get from `ExportRuntime::shared`.

## Hot-path memory model (observability-core)

`observability-core` writes only thread-local state on the hot path; its one shared read is the gate flag. Everything sampled per-message lives in thread-local state, handed to one shared slot per thread only on a periodic tick or at thread exit.

The runtime gate is one private process-wide `AtomicBool`. [[crates/observability-core/src/gate.rs#set_metrics_enabled]] writes it and [[crates/observability-core/src/gate.rs#metrics_enabled]] reads it with a `Relaxed` load on every call, with no per-thread copy. The flag is written rarely and stays in each core's cache, so the load costs the same as a thread-local read. A write is visible on every thread without a refresh call.

Latency samples land in a thread-local bounded `hdrhistogram::Histogram` (1ns..60s, 3 sig figs, `saturating_record` so a tail spike clamps into range instead of panicking) via [[crates/observability-core/src/handoff.rs#record_latency_ns]]. The histogram is allocated on a thread's first sample, so a thread that only merges never allocates one. [[crates/observability-core/src/handoff.rs#merge_local]] hands that thread's samples, plus its message count, to the thread's one shared slot, registering the slot on the first hand-off with data; a merge with nothing pending touches no shared state. When the thread exits, its thread-local destructor hands off whatever it has not merged and marks the slot retired. [[crates/observability-core/src/handoff.rs#drain_into]] resets a caller-owned [[crates/observability-core/src/handoff.rs#LatencyWindow]], folds every slot into it, and clears each slot in place. It removes a slot once it has read the retired flag before draining that slot, so thread churn does not grow memory and a flusher only ever sees data a worker has already handed off.

Message counting mirrors the same shape: a plain per-thread `Cell<u64>` on the hot path (`count_msg`/`take_count`), never a shared atomic, handed to the same slot as the histogram on the same `merge_local` tick or at thread exit. The first `count_msg` after a reset also touches the thread-local state once, so a thread that only counts still hands its count off at exit.

[[crates/observability-core/src/sample.rs#should_sample]] is a per-thread wraparound sampler (`mask` must be `2^n - 1`, e.g. `SAMPLE_1_IN_8192`) for hot-path call sites that need periodic-not-every-message sampling. It samples true on a fresh thread's very first call, then exactly once per `mask + 1` consecutive calls after that.

[[crates/observability-core/benches/hot_path.rs]] measures these calls with std and `hdrhistogram` only. It prints batch-mean nanoseconds per call (p50, p99, p999, max) for each hot-path call and for drains over 1, 8, and 64 live slots, then the RSS growth and drain time after 2,000 short-lived recording threads. Run it with `cargo bench -p observability-core --bench hot_path`.

## Shared OTLP, three sibling exporters

[[crates/observability/src/otlp/mod.rs#OtlpConfig]] is the single source of endpoint, protocol, headers, resource attrs, timeout, and TLS for every OTLP signal.

Its `Debug` output prints each header name with `[REDACTED]` in place of the value, because header values usually carry API keys. `PipelineConfig` derives `Debug` and embeds the `OtlpConfig`, so `?cfg` in a log line is safe for both.

`pipeline/otel.rs` and `metrics.rs` each call its exporter builders directly: `span_exporter` for tracing, `metric_exporter` (under `metrics-otel`) for the metrics bridge, `log_exporter` (under `otel-logs`) for the log bridge layer, all off the same endpoint/timeout/headers pair. No `opentelemetry` type crosses into a public `observability` signature; the builders stay `pub(crate)` and otel-gated.

[[crates/observability/src/otlp/runtime.rs#ExportRuntime]] is the one tokio runtime (multi-thread, 1 worker, threads named `observability-otlp`) that runs every exporter's I/O, so OTLP export works with or without an app runtime. gRPC exporters are built inside its context, so tonic spawns its channel worker there. HTTP exporters send through [[crates/observability/src/otlp/http_client.rs#RuntimeHttpClient]], an async reqwest client whose requests are spawned onto that runtime.

Dropping a guard shuts its OTel providers down, which exports the final batch, and logs a failed shutdown as a warning. Export runs on the export runtime, so this works from a thread with no runtime and from inside a current_thread or multi-thread app runtime, async context included. `ObsGuard` declares `metrics` before `pipeline`. The metrics recorder therefore shuts down first, while the log sinks can still print its warnings.

`ExportRuntime::shared` hands out the live runtime through a `Weak` slot, so the pipeline and metrics guards share one export thread, and an init after every guard dropped starts a fresh one. Each guard declares its `Arc` after its providers, which keeps the runtime running through their final flush. The last `Arc` to drop calls `shutdown_background`, which is safe inside an async context.

Every configured header goes out on both transports: as HTTP headers, or as gRPC metadata through `MetadataMap::from_headers`. A header whose name or value is not valid HTTP returns `OtlpError::InvalidHeader` at init, naming the header and never its value, because opentelemetry-otlp would drop it without a word.

TLS applies only to `https://` endpoints, so `http://localhost` stays plain text. [[crates/observability/src/otlp/tls.rs#OtlpTls]] holds the TLS settings, and the default `tls` feature turns TLS on. `OtlpTls::roots` selects the OS store (`Native`, checked at init for at least one certificate so both transports fail the same way when it is empty), the bundled Mozilla set (`Webpki`, under `tls-webpki-roots`), only `ca_file` (`CaFileOnly`), or the OS verifier through `rustls-platform-verifier` (`Platform`, under `tls-platform-verifier`, for desktop and mobile apps). HTTP passes a finished rustls config to reqwest's `use_preconfigured_tls`. gRPC builds its own tonic `Channel` with `Endpoint::tls_config_with_verifier` and hands it to opentelemetry-otlp's `with_channel`. `ca_file` adds its CA certificates to any roots choice, and `client_cert` presents a certificate for mutual TLS. Both files are read and parsed at init, so a missing or malformed file returns `OtlpError::TlsFile` or `OtlpError::InvalidPem` with its path on either transport. Without `tls`, an `https://` endpoint returns `OtlpError::FeatureDisabled`. ring is the crypto provider unless the app installs a process-wide one first.

`otel_resource` folds `service.name` plus every configured `resource` attribute onto whichever provider calls it, so span, metric, and log all carry identical resource attributes from one config, with no per-signal drift.

## Feature model

`otel` is the grouping feature. Per-signal features layer on top of it.

`otel` pulls in `opentelemetry`/`opentelemetry_sdk`/`opentelemetry-otlp`/`tracing-opentelemetry`, plus `tokio` and `reqwest` for the export runtime, and gates every `#[cfg(feature = "otel")]` block. `tls` (default) adds rustls with ring for tonic and reqwest and the OS store check; `tls-webpki-roots` adds the bundled Mozilla roots; `tls-platform-verifier` adds `rustls-platform-verifier`. `otel-traces` is a plain alias for `otel`. `otel-logs` adds the OTLP log bridge (`opentelemetry-appender-tracing`). `metrics-prom` adds the Prometheus pull exporter. `metrics-otel` adds the OTLP metrics bridge and implies `otel`. `bunyan` adds the bunyan-format log sink.

No feature is named `tracing` or `metrics`; those names stay reserved for the extern facade crate deps themselves, so a feature flag never shadows `tracing::` or `metrics::` path resolution.

The whole otel stack pins to opentelemetry 0.31: `metrics-exporter-otel` 0.3.1 pins `opentelemetry ^0.31`, and all three OTLP signal exporters plus the metrics bridge must resolve to one `opentelemetry` version, so that crate is the version bottleneck for the entire stack.

## See also

Related sections outside this file:

- [[tests]] documents test specifications for the runtime gate, sampler, per-thread counters, and histogram merge.
